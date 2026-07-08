//! Pass 2 — type-check dos corpos, inferência, dispatch por dominância.
//!
//! Consome `ResolvedModule` (TypeEnv + assinaturas + EnumRegistry) + `Module` (AST) e
//! produz `TypedModule` (TAST com `ty`, `tail_pos`, `effect` em cada nó).
//!
//! Algoritmo: `infer_module` popula o DispatchTable a partir das
//! `signatures`, depois `infer_expr` percorre a AST recursivamente,
//! despachando `Apply` via `DispatchTable::resolve` ou `call_indirect`
//! via `TypeEnv` lookup.

use kata_ast::{
    Expr, GuardClause, Item, MatchArm, Module, Pattern, Span, Spanned, TypeExpr, WithBinding,
};
use kata_core::dispatch::{DispatchError, DispatchTable, OverloadInfo};
use kata_core::enum_registry::EnumRegistry;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::{ResolvedModule, Signature};

use crate::desugar;
use crate::patterns;
use crate::typed::{
    Effect, TypedExpr, TypedExprKind, TypedGuardClause, TypedLambdaClause,
    TypedMatchArm, TypedModule, TypedPattern, TypedWithBinding,
};

/// Erro de inferência — wrapped `MiddleError` (carrega Span).
pub type InferResult<T> = Result<T, MiddleError>;

/// Popula o DispatchTable a partir das assinaturas do ResolvedModule.
fn populate_dispatch_table(signatures: &[Signature]) -> DispatchTable {
    let mut table = DispatchTable::new();
    for sig in signatures {
        let ffi_symbol = sig.ffi_symbol.clone();
        let is_associative = sig.is_associative;
        let associative_neutral = sig.associative_neutral;

        table.insert(OverloadInfo {
            name: sig.name.clone(),
            params: sig.param_types.clone(),
            ret: sig.return_type.clone(),
            ffi_symbol,
            is_action: false,
            is_generic: false,
            is_constructor: false,
            associative_neutral,
        });

        // Marca comutativa para operadores associativos (+, *)
        if is_associative && sig.name.len() == 1 {
            let c = sig
                .name
                .chars()
                .next()
                .expect("nome de operador tem 1 char");
            if c == '+' || c == '*' {
                table.mark_commutative(&sig.name);
            }
        }
    }
    table
}

/// Infere o tipo de um módulo completo.
///
/// Pipeline: popula DispatchTable → percorre items → infere entry point.
/// Retorna `TypedModule` ou o primeiro erro de typeck encontrado.
pub fn infer_module(module: &Module, resolved: &ResolvedModule) -> InferResult<TypedModule> {
    // 1. Popula DispatchTable com as assinaturas (prelude + módulo)
    let dispatch_table = populate_dispatch_table(&resolved.signatures);

    // 2. Clona o TypeEnv do ResolvedModule — o typeck pode adicionar bindings
    //    locais (let) sem mutar o original.
    let mut type_env = resolved.type_env.clone();

    // 3. Percorre items — Sigs e decls de tipo já foram processados no
    //    resolution. Aqui só processamos EntryExpr (a última expr).
    let mut entry_expr: Option<Spanned<TypedExpr>> = None;

    for item in &module.items {
        match &item.node {
            Item::EntryExpr(expr) => {
                // Desugar Pipe e Hole antes do typeck. Após isto, a AST
                // não contém Expr::Pipe nem Expr::Hole — o typeck nunca os
                // vê. Isto é total: TAST nunca contém Pipe nem Hole.
                let desugared = desugar::desugar(expr);
                let typed = infer_expr(
                    &desugared.node,
                    &desugared.span,
                    &mut type_env,
                    &dispatch_table,
                    &resolved.enum_registry,
                    true, // entry point está em tail position
                )?;
                entry_expr = Some(Spanned::new(typed, expr.span));
            }
            Item::Sig { .. } | Item::DataDecl { .. } | Item::EnumDecl { .. } => {
                // Já processado no resolution. Nada a fazer no inference.
            }
        }
    }

    let entry = entry_expr.ok_or_else(|| MiddleError::UnboundName {
        name: "<entry point>".into(),
        span: item_span_or_synthetic(&module.items),
    })?;

    Ok(TypedModule {
        entry,
        dispatch_table,
        type_env,
    })
}

/// Span do último item ou sintético se módulo vazio.
fn item_span_or_synthetic(items: &[Spanned<Item>]) -> kata_diagnostics::MietteSpan {
    items
        .last()
        .map(|i| i.span.into())
        .unwrap_or(kata_diagnostics::MietteSpan(Span::synthetic()))
}

/// Infere o tipo de uma expressão, produzindo um `TypedExpr`.
///
/// `tail_pos` é `true` quando a expressão está em posição de cauda. O entry
/// point é sempre `tail_pos = true`. Sub-expressões de `Let` value são
/// `tail_pos = false`. Argumentos de `Apply` são `tail_pos = false`.
/// Body de lambda em tail position é `tail_pos = true`. Body de match arm
/// em tail position é `tail_pos = true`.
fn infer_expr(
    expr: &Expr,
    span: &Span,
    env: &mut TypeEnv,
    table: &DispatchTable,
    enum_registry: &EnumRegistry,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let (ty, kind, effect) = match expr {
        // ── Literais ─────────────────────────────────────────
        Expr::IntLit { text } => (
            Ty::int(),
            TypedExprKind::IntLit { text: text.clone() },
            Effect::Puro,
        ),
        Expr::FloatLit { text } => (
            Ty::float(),
            TypedExprKind::FloatLit { text: text.clone() },
            Effect::Puro,
        ),
        Expr::TextLit { text } => (
            Ty::text(),
            TypedExprKind::TextLit { text: text.clone() },
            Effect::Puro,
        ),
        Expr::Unit => (Ty::Unit, TypedExprKind::Unit, Effect::Puro),

        // ── Identificador ────────────────────────────────────
        Expr::Ident { name } => {
            let ty = env
                .lookup(name)
                .cloned()
                .ok_or_else(|| MiddleError::UnboundName {
                    name: name.clone(),
                    span: (*span).into(),
                })?;
            (
                ty,
                TypedExprKind::Ident { name: name.clone() },
                Effect::Puro,
            )
        }

        // ── Aplicação prefixa ────────────────────────────────
        Expr::Apply { callee, args } => {
            infer_apply(callee, args, span, env, table, enum_registry, tail_pos)?
        }

        // ── Ascription de tipo ───────────────────────────────
        Expr::TypeAscription { expr, ty } => {
            let inner = infer_expr(&expr.node, &expr.span, env, table, enum_registry, false)?;
            let target_ty = resolve_type_expr(&ty.node, env);

            let rebaixa_ok = match (&inner.kind, &target_ty) {
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Int)) => true,
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Float)) => true,
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
                (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Float)) => true,
                (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
                (TypedExprKind::TextLit { .. }, Ty::Prim(PrimTy::Text)) => true,
                _ if inner.ty == target_ty => true,
                _ => false,
            };

            if !rebaixa_ok {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{:?}", target_ty),
                    found: format!("{:?}", inner.ty),
                    span: expr.span.into(),
                });
            }

            (
                target_ty.clone(),
                TypedExprKind::TypeAscription {
                    expr: Box::new(Spanned::new(inner, expr.span)),
                    target_ty,
                },
                Effect::Puro,
            )
        }

        // ── Grouping — transparente ─────────────────────────
        Expr::Grouping { inner } => {
            let typed_inner = infer_expr(&inner.node, &inner.span, env, table, enum_registry, tail_pos)?;
            (
                typed_inner.ty.clone(),
                TypedExprKind::Grouping {
                    inner: Box::new(Spanned::new(typed_inner, inner.span)),
                },
                Effect::Puro,
            )
        }

        // ── Tuple ────────────────────────────────────────────
        Expr::Tuple { elements } => {
            let mut typed_elements = Vec::with_capacity(elements.len());
            let mut element_tys = Vec::with_capacity(elements.len());
            for elem in elements {
                let typed = infer_expr(&elem.node, &elem.span, env, table, enum_registry, false)?;
                element_tys.push(typed.ty.clone());
                typed_elements.push(Spanned::new(typed, elem.span));
            }
            (
                Ty::Tuple(element_tys),
                TypedExprKind::Tuple {
                    elements: typed_elements,
                },
                Effect::Puro,
            )
        }

        // ── Let binding ──────────────────────────────────────
        Expr::Let { name, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, table, enum_registry, false)?;
            let val_ty = typed_value.ty.clone();

            env.define(name, val_ty);

            (
                Ty::Unit,
                TypedExprKind::Let {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
                Effect::Puro,
            )
        }

        // ── Qualificação de variante ─────────────────────────
        Expr::VariantQual { enum_name, variant } => {
            let enum_ty = env
                .lookup(enum_name)
                .cloned()
                .ok_or_else(|| MiddleError::UnboundName {
                    name: enum_name.clone(),
                    span: (*span).into(),
                })?;

            match &enum_ty {
                Ty::Sum(name) => {
                    let _ = variant;
                    (
                        enum_ty.clone(),
                        TypedExprKind::VariantQual {
                            enum_name: name.clone(),
                            variant: variant.clone(),
                        },
                        Effect::Puro,
                    )
                }
                _ => Err(MiddleError::TypeMismatch {
                    expected: "enum".to_string(),
                    found: format!("{:?}", enum_ty),
                    span: (*span).into(),
                })?,
            }
        }

        // ── Fio 2: desugared antes do typeck ──────────────────
        Expr::Hole => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (Hole deve ter sido desugared)".into(),
                found: "Hole".into(),
                span: (*span).into(),
            });
        }
        Expr::Pipe { .. } => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (Pipe deve ter sido desugared)".into(),
                found: "Pipe".into(),
                span: (*span).into(),
            });
        }

        // ── Fio 2 Fase 8: Lambda ──────────────────────────────
        Expr::Lambda {
            patterns,
            body,
            guards,
            with_bindings,
        } => infer_lambda(
            patterns,
            body,
            guards,
            with_bindings,
            span,
            env,
            table,
            enum_registry,
            tail_pos,
        )?,

        // ── Fio 2 Fase 8: Match ───────────────────────────────
        Expr::Match { scrutinee, arms } => {
            infer_match(scrutinee, arms, span, env, table, enum_registry, tail_pos)?
        }
    };

    Ok(TypedExpr {
        span: *span,
        ty,
        tail_pos,
        effect,
        kind,
    })
}

/// Infere uma aplicação prefixa — dois caminhos de callee (Fio 2).
///
/// 1. Callee é nome no DispatchTable: `table.resolve(name, arg_types)` → call direto.
/// 2. Callee é variável no TypeEnv com `Ty::Function`: `call_indirect` no codegen.
///
/// DispatchTable vence se encontrado em ambos (call direto é mais eficiente).
fn infer_apply(
    callee: &Spanned<Expr>,
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    table: &DispatchTable,
    enum_registry: &EnumRegistry,
    _tail_pos: bool,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    let func_name = match &callee.node {
        Expr::Ident { name } => name.clone(),
        _ => {
            return Err(MiddleError::UnboundName {
                name: "<non-ident callee>".into(),
                span: callee.span.into(),
            });
        }
    };

    // Infere tipos dos argumentos recursivamente (tail_pos = false para args).
    let mut typed_args: Vec<Spanned<TypedExpr>> = Vec::with_capacity(args.len());
    let mut arg_types: Vec<Ty> = Vec::with_capacity(args.len());

    for arg in args {
        let typed = infer_expr(&arg.node, &arg.span, env, table, enum_registry, false)?;
        arg_types.push(typed.ty.clone());
        typed_args.push(Spanned::new(typed, arg.span));
    }

    // Caminho 1: DispatchTable (call direto para FFI ou função Kata nomeada).
    if table.has_function(&func_name) {
        let overload = table
            .resolve(&func_name, &arg_types)
            .map_err(|e| dispatch_to_middle_error(e, *span))?;

        let callee_ty = Ty::Function(overload.params.clone(), Box::new(overload.ret.clone()));
        let callee_typed = TypedExpr {
            span: callee.span,
            ty: callee_ty,
            tail_pos: false,
            effect: Effect::Puro,
            kind: TypedExprKind::Ident {
                name: func_name.clone(),
            },
        };

        return Ok((
            overload.ret,
            TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee_typed, callee.span)),
                args: typed_args,
                ffi_symbol: overload.ffi_symbol,
                captures: Vec::new(),
                escapes: false,
            },
            Effect::Puro,
        ));
    }

    // Caminho 2: TypeEnv (call_indirect para lambda como valor).
    if let Some(Ty::Function(param_types, ret_ty)) = env.lookup(&func_name).cloned() {
        // Verifica aridade.
        if arg_types.len() != param_types.len() {
            return Err(MiddleError::ArityMismatch {
                expected: param_types.len(),
                found: arg_types.len(),
                span: (*span).into(),
            });
        }
        // Verifica tipos dos argumentos.
        for (i, (arg_ty, param_ty)) in arg_types.iter().zip(param_types.iter()).enumerate() {
            if arg_ty != param_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{:?}", param_ty),
                    found: format!("{:?}", arg_ty),
                    span: args[i].span.into(),
                });
            }
        }

        let callee_typed = TypedExpr {
            span: callee.span,
            ty: Ty::Function(param_types.clone(), ret_ty.clone()),
            tail_pos: false,
            effect: Effect::Puro,
            kind: TypedExprKind::Ident {
                name: func_name.clone(),
            },
        };

        return Ok((
            (*ret_ty).clone(),
            TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee_typed, callee.span)),
                args: typed_args,
                ffi_symbol: None, // call_indirect — sem FFI symbol
                captures: Vec::new(),
                escapes: false,
            },
            Effect::Puro,
        ));
    }

    // Não encontrado em nenhum lugar.
    Err(MiddleError::UnboundName {
        name: func_name,
        span: callee.span.into(),
    })
}

/// Infere um lambda anônimo ou cláusula lambda.
///
/// Para lambda anônimo: 1 cláusula, sem nome de função.
/// Para função nomeada (cláusulas de Sig): múltiplas cláusulas, com nome.
///
/// Em Fio 2, lambda anônimo não tem assinatura — os tipos dos parâmetros
/// são inferidos a partir do primeiro uso. Para funções nomeadas, a
/// assinatura fornece os tipos. Aqui tratamos apenas lambda anônimo
/// (sem assinatura); funções nomeadas são tratadas no resolution/inference
/// do Sig (Fase 10).
fn infer_lambda(
    patterns: &[Spanned<Pattern>],
    body: &Spanned<Expr>,
    guards: &[GuardClause],
    with_bindings: &[WithBinding],
    span: &Span,
    env: &mut TypeEnv,
    table: &DispatchTable,
    enum_registry: &EnumRegistry,
    tail_pos: bool,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    // Para lambda anônimo, os tipos dos parâmetros são InferVar — não temos
    // inferência de tipos real ainda. Em Fio 2, o lambda anônimo só funciona
    // quando o tipo é determinado pelo contexto (ex: `let f := lambda x: + x 1`
    // infere x:Int porque + exige Int). Mas sem inferência bidirecional, isso
    // não é possível. Fio 2 usa uma abordagem simples: InferVar e unificação
    // não estão implementados — o lambda ganha tipos InferVar e o primeiro
    // uso determina o tipo.
    //
    // Para Fase 8, implementamos a estrutura do TypedLambdaClause mas a
    // inferência de tipos do lambda é limitada: cada padrão Ident ganha
    // InferVar e o body é inferido no escopo. O tipo de retorno é o tipo
    // do body. O tipo do lambda é Function(param_types, ret_ty).

    // Cria escopo filho para os bindings do lambda.
    let mut lambda_env = env.push_scope();

    // Processa padrões — cada Ident ganha InferVar como tipo placeholder.
    // Sem inferência bidirecional, usamos InferVar fresco para cada parâmetro.
    let mut param_types: Vec<Ty> = Vec::with_capacity(patterns.len());
    let mut typed_patterns: Vec<Spanned<TypedPattern>> = Vec::with_capacity(patterns.len());

    for (i, pat) in patterns.iter().enumerate() {
        let param_ty = Ty::InferVar(i as u32);
        let typed_pat = patterns::check_pattern(pat, &param_ty, enum_registry, &mut lambda_env)?;
        param_types.push(param_ty);
        typed_patterns.push(typed_pat);
    }

    // Processa with bindings (açúcar → let chain no escopo do lambda).
    // with bindings são pré-avaliados antes dos guards.
    let mut typed_with_bindings: Vec<TypedWithBinding> = Vec::new();
    for wb in with_bindings {
        let typed_value = infer_expr(&wb.value.node, &wb.value.span, &mut lambda_env, table, enum_registry, false)?;
        let val_ty = typed_value.ty.clone();
        lambda_env.define(&wb.name, val_ty);
        typed_with_bindings.push(TypedWithBinding {
            name: wb.name.clone(),
            value: Spanned::new(typed_value, wb.value.span),
        });
    }

    // Infere o corpo do lambda.
    // Se há guards, o corpo é decidido pelos guards — cada guard é um
    // TypedGuardClause. O tipo de retorno é o tipo do body de qualquer
    // guard (todos devem concordar — verificação futura).
    // Se não há guards, o body é a expressão única após `:`.
    let mut typed_guards: Vec<TypedGuardClause> = Vec::new();
    let (ret_ty, typed_body) = if guards.is_empty() {
        let typed_body = infer_expr(&body.node, &body.span, &mut lambda_env, table, enum_registry, tail_pos)?;
        (typed_body.ty.clone(), typed_body)
    } else {
        let mut guard_ret_ty: Option<Ty> = None;
        for guard in guards {
            let guard_body_typed = if let Some(cond) = &guard.condition {
                // Guard com condição: infere condição (deve ser Boolean) e body.
                let cond_typed = infer_expr(&cond.node, &cond.span, &mut lambda_env, table, enum_registry, false)?;
                // Verifica que a condição é Boolean.
                if cond_typed.ty != Ty::boolean() {
                    return Err(MiddleError::TypeMismatch {
                        expected: "Boolean".into(),
                        found: format!("{:?}", cond_typed.ty),
                        span: cond.span.into(),
                    });
                }
                let body_typed = infer_expr(&guard.body.node, &guard.body.span, &mut lambda_env, table, enum_registry, tail_pos)?;
                if let Some(ref existing) = guard_ret_ty {
                    if *existing != body_typed.ty {
                        return Err(MiddleError::TypeMismatch {
                            expected: format!("{:?}", existing),
                            found: format!("{:?}", body_typed.ty),
                            span: guard.body.span.into(),
                        });
                    }
                } else {
                    guard_ret_ty = Some(body_typed.ty.clone());
                }
                TypedGuardClause {
                    condition: Some(Spanned::new(cond_typed, cond.span)),
                    body: Spanned::new(body_typed, guard.body.span),
                }
            } else {
                // otherwise: body sem condição.
                let body_typed = infer_expr(&guard.body.node, &guard.body.span, &mut lambda_env, table, enum_registry, tail_pos)?;
                if let Some(ref existing) = guard_ret_ty {
                    if *existing != body_typed.ty {
                        return Err(MiddleError::TypeMismatch {
                            expected: format!("{:?}", existing),
                            found: format!("{:?}", body_typed.ty),
                            span: guard.body.span.into(),
                        });
                    }
                } else {
                    guard_ret_ty = Some(body_typed.ty.clone());
                }
                TypedGuardClause {
                    condition: None,
                    body: Spanned::new(body_typed, guard.body.span),
                }
            };
            typed_guards.push(guard_body_typed);
        }
        let rt = guard_ret_ty.ok_or_else(|| MiddleError::TypeMismatch {
            expected: "pelo menos um guard".into(),
            found: "nenhum guard".into(),
            span: (*span).into(),
        })?;
        // Body é ignorado quando há guards — usamos o body do último guard
        // como placeholder. O codegen decide pelos guards.
        // Construímos um TypedExpr placeholder a partir do body original.
        let placeholder_body = TypedExpr {
            span: body.span,
            ty: rt.clone(),
            tail_pos,
            effect: Effect::Puro,
            kind: TypedExprKind::Unit,
        };
        (rt, placeholder_body)
    };

    let lambda_ty = Ty::Function(param_types.clone(), Box::new(ret_ty.clone()));

    let clause = TypedLambdaClause {
        patterns: typed_patterns,
        body: Spanned::new(typed_body, body.span),
        guards: typed_guards,
        with_bindings: typed_with_bindings,
    };

    Ok((
        lambda_ty,
        TypedExprKind::Lambda {
            func_name: None, // lambda anônimo — Fase 10 atribui nome para Sig
            param_types,
            ret_ty,
            clauses: vec![clause],
        },
        Effect::Puro,
    ))
}

/// Infere um `match` — pattern matching com verificação de exaustividade.
fn infer_match(
    scrutinee: &Spanned<Expr>,
    arms: &[MatchArm],
    span: &Span,
    env: &mut TypeEnv,
    table: &DispatchTable,
    enum_registry: &EnumRegistry,
    tail_pos: bool,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    // Infere o scrutinee.
    let typed_scrutinee = infer_expr(&scrutinee.node, &scrutinee.span, env, table, enum_registry, false)?;
    let scrutinee_ty = typed_scrutinee.ty.clone();

    // Processa cada braço.
    let mut typed_arms: Vec<TypedMatchArm> = Vec::with_capacity(arms.len());
    let mut match_ret_ty: Option<Ty> = None;
    let mut covered_variants: Vec<String> = Vec::new();
    let mut has_otherwise = false;

    for arm in arms {
        // Cria escopo filho para bindings do pattern.
        let mut arm_env = env.push_scope();

        let typed_pattern = if let Some(pat) = &arm.pattern {
            let typed_pat = patterns::check_pattern(pat, &scrutinee_ty, enum_registry, &mut arm_env)?;
            // Coleta variantes cobertas para exaustividade.
            if let TypedPattern::Variant { variant, .. } = &typed_pat.node {
                covered_variants.push(variant.clone());
            }
            // Ident e Wildcard cobrem qualquer valor — contam como fallback.
            if matches!(
                &typed_pat.node,
                TypedPattern::Ident { .. } | TypedPattern::Wildcard
            ) {
                has_otherwise = true;
            }
            Some(typed_pat)
        } else {
            // otherwise — pattern None.
            has_otherwise = true;
            None
        };

        // Infere guard (se houver).
        let typed_guard = if let Some(guard_expr) = &arm.guard {
            let guard_typed = infer_expr(&guard_expr.node, &guard_expr.span, &mut arm_env, table, enum_registry, false)?;
            if guard_typed.ty != Ty::boolean() {
                return Err(MiddleError::TypeMismatch {
                    expected: "Boolean".into(),
                    found: format!("{:?}", guard_typed.ty),
                    span: guard_expr.span.into(),
                });
            }
            Some(Spanned::new(guard_typed, guard_expr.span))
        } else {
            None
        };

        // Infere body do braço.
        let typed_body = infer_expr(&arm.body.node, &arm.body.span, &mut arm_env, table, enum_registry, tail_pos)?;

        // Verifica que todos os braços retornam o mesmo tipo.
        if let Some(ref existing) = match_ret_ty {
            if *existing != typed_body.ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{:?}", existing),
                    found: format!("{:?}", typed_body.ty),
                    span: arm.body.span.into(),
                });
            }
        } else {
            match_ret_ty = Some(typed_body.ty.clone());
        }

        typed_arms.push(TypedMatchArm {
            pattern: typed_pattern,
            guard: typed_guard,
            body: Spanned::new(typed_body, arm.body.span),
        });
    }

    let ret_ty = match_ret_ty.ok_or_else(|| MiddleError::TypeMismatch {
        expected: "pelo menos um braço".into(),
        found: "nenhum braço".into(),
        span: (*span).into(),
    })?;

    // Verifica exaustividade.
    patterns::check_exhaustiveness(
        &covered_variants,
        &scrutinee_ty,
        has_otherwise,
        enum_registry,
        span,
    )?;

    Ok((
        ret_ty.clone(),
        TypedExprKind::Match {
            scrutinee: Box::new(Spanned::new(typed_scrutinee, scrutinee.span)),
            arms: typed_arms,
        },
        Effect::Puro,
    ))
}

/// Converte `DispatchError` em `MiddleError` para diagnóstico.
fn dispatch_to_middle_error(err: DispatchError, span: Span) -> MiddleError {
    match err {
        DispatchError::FunctionNotFound { name, .. } => MiddleError::NoOverload {
            name,
            span: span.into(),
        },
        DispatchError::TypeMismatch { name, .. } => MiddleError::NoOverload {
            name,
            span: span.into(),
        },
        DispatchError::AmbiguousDispatch { name, .. } => MiddleError::AmbiguousDispatch {
            name,
            span: span.into(),
        },
    }
}

/// Resolve `TypeExpr` → `Ty` usando o TypeEnv. Igual ao `resolve_type_expr`
/// do resolution, mas replicado aqui para evitar depender de função privada.
fn resolve_type_expr(expr: &TypeExpr, env: &TypeEnv) -> Ty {
    match expr {
        TypeExpr::Named(name) => {
            if let Some(ty) = env.lookup(name) {
                ty.clone()
            } else {
                match name.as_str() {
                    "Int" => Ty::Prim(PrimTy::Int),
                    "Float" => Ty::Prim(PrimTy::Float),
                    "Text" => Ty::Prim(PrimTy::Text),
                    "Rational" => Ty::Prim(PrimTy::Rational),
                    "Boolean" => Ty::Sum("Boolean".into()),
                    "Unit" => Ty::Unit,
                    _ => Ty::Struct(name.clone()),
                }
            }
        }
        TypeExpr::Unit => Ty::Unit,
        TypeExpr::Grouping(inner) => resolve_type_expr(&inner.node, env),
        TypeExpr::Func { params, ret } => {
            let param_types: Vec<Ty> = params
                .iter()
                .map(|t| resolve_type_expr(&t.node, env))
                .collect();
            let return_type = resolve_type_expr(&ret.node, env);
            Ty::Function(param_types, Box::new(return_type))
        }
        TypeExpr::ParamApp { name, .. } => Ty::Sum(name.clone()),
    }
}