//! Pass 2 — type-check dos corpos, inferência, dispatch por dominância.
//!
//! Consome `ResolvedModule` (TypeEnv + assinaturas) + `Module` (AST) e
//! produz `TypedModule` (TAST com `ty`, `tail_pos`, `effect` em cada nó).
//!
//! Algoritmo: `infer_module` popula o DispatchTable a partir das
//! `signatures`, depois `infer_expr` percorre a AST recursivamente,
//! despachando `Apply` via `DispatchTable::resolve`.

use kata_ast::{Expr, Item, Module, Span, Spanned, TypeExpr};
use kata_core::dispatch::{DispatchError, DispatchTable, OverloadInfo};
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::{ResolvedModule, Signature};

use crate::typed::{Effect, TypedExpr, TypedExprKind, TypedModule};

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
            // Heurística simples: operadores de 1 char que são associativos
            // são comutativos para efeito de dispatch (tentar args invertidos).
            // Em Fio 1, + e * são comutativos; - e / não.
            let c = sig.name.chars().next().unwrap();
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
                let typed = infer_expr(&expr.node, &expr.span, &mut type_env, &dispatch_table)?;
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
/// `tail_pos = false`. Em Fio 1, sem blocos/lambdas, a propagação é trivial.
fn infer_expr(
    expr: &Expr,
    span: &Span,
    env: &mut TypeEnv,
    table: &DispatchTable,
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
            // O callee deve ser um Ident (nome de função) em Fio 1.
            // Aplicação de valor não-função não existe ainda (sem lambdas).
            let func_name = match &callee.node {
                Expr::Ident { name } => name.clone(),
                _ => {
                    return Err(MiddleError::UnboundName {
                        name: "<non-ident callee>".into(),
                        span: callee.span.into(),
                    });
                }
            };

            // Infere tipos dos argumentos recursivamente
            let mut typed_args: Vec<Spanned<TypedExpr>> = Vec::with_capacity(args.len());
            let mut arg_types: Vec<Ty> = Vec::with_capacity(args.len());

            for arg in args {
                let typed = infer_expr(&arg.node, &arg.span, env, table)?;
                arg_types.push(typed.ty.clone());
                typed_args.push(Spanned::new(typed, arg.span));
            }

            // Despacha via DispatchTable
            let overload = table
                .resolve(&func_name, &arg_types)
                .map_err(|e| dispatch_to_middle_error(e, *span))?;

            // O callee é um Ident cujo tipo é a função despachada.
            // Não chamamos infer_expr no callee — ele é um nome de função,
            // não uma variável no TypeEnv. Construímos o TypedExpr diretamente.
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

            (
                overload.ret,
                TypedExprKind::Apply {
                    callee: Box::new(Spanned::new(callee_typed, callee.span)),
                    args: typed_args,
                    ffi_symbol: overload.ffi_symbol,
                },
                Effect::Puro,
            )
        }

        // ── Ascription de tipo ───────────────────────────────
        Expr::TypeAscription { expr, ty } => {
            let inner = infer_expr(&expr.node, &expr.span, env, table)?;
            let target_ty = resolve_type_expr(&ty.node, env);

            // Valida compatibilidade. Rebaixamento só se aplica a literais:
            // o literal é reinterpretado no tipo alvo desde o início (sem
            // conversão em runtime). Para não-literais, ascription é
            // no-op (mesmo tipo) ou erro (use a função de conversão).
            //
            // Rebaixamentos válidos em Fio 1:
            //   - IntLit  → Int, Float, Rational  (texto bruto reinterpretado)
            //   - FloatLit → Float, Rational      (texto bruto reinterpretado)
            //   - TextLit  → Text                  (no-op, mesmo tipo)
            //
            // O codegen inspeciona (inner.kind, target_ty) para decidir
            // o símbolo FFI: IntLit→Float = f64 const, IntLit→Rational =
            // kata_rt_rat_literal, FloatLit→Rational = kata_rt_rat_literal.
            let rebaixa_ok = match (&inner.kind, &target_ty) {
                // IntLit rebaixa para Int (no-op), Float, Rational
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Int)) => true,
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Float)) => true,
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
                // FloatLit rebaixa para Float (no-op), Rational
                (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Float)) => true,
                (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
                // TextLit rebaixa para Text (no-op)
                (TypedExprKind::TextLit { .. }, Ty::Prim(PrimTy::Text)) => true,
                // Demais casos: mesmo tipo (no-op) é OK
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
            let typed_inner = infer_expr(&inner.node, &inner.span, env, table)?;
            (
                typed_inner.ty.clone(),
                TypedExprKind::Grouping {
                    inner: Box::new(Spanned::new(typed_inner, inner.span)),
                },
                Effect::Puro,
            )
        }

        // ── Tuple — NÃO suportado em Fio 1 ───────────────────
        // Ty::Tuple não existe (Fio 5). O typeck rejeita com erro limpo.
        Expr::Tuple { elements } => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão não-tupla (tuples são Fio 5)".into(),
                found: format!("tupla com {} elemento(s)", elements.len()),
                span: (*span).into(),
            });
        }

        // ── Let binding ──────────────────────────────────────
        Expr::Let { name, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, table)?;
            let val_ty = typed_value.ty.clone();

            // Define o nome no escopo atual
            env.define(name, val_ty);

            (
                Ty::Unit, // let retorna Unit
                TypedExprKind::Let {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
                Effect::Puro,
            )
        }

        // ── Qualificação de variante ─────────────────────────
        Expr::VariantQual { enum_name, variant } => {
            // Verifica que o enum existe no TypeEnv
            let enum_ty =
                env.lookup(enum_name)
                    .cloned()
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: enum_name.clone(),
                        span: (*span).into(),
                    })?;

            match &enum_ty {
                Ty::Sum(name) => {
                    // Variante de enum. Em Fio 1, variantes são unitárias
                    // (Boolean::True, Boolean::False). O tipo é o Sum.
                    let _ = variant; // Fio 4 validará a variante existe
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
    };

    // Em Fio 1, toda expressão é pura. tail_pos é marcado pelo chamador
    // (entry point = true, sub-expressões de Let value = false).
    // Aqui marcamos true por padrão — o codegen/optimizer ajustará.
    Ok(TypedExpr {
        span: *span,
        ty,
        tail_pos: true,
        effect,
        kind,
    })
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
