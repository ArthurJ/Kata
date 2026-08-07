//! Núcleo da inferência de expressões — o grande match sobre `Expr`.
//!
//! `infer_expr` é o entry point público (usado por todos os submódulos).
//! `infer_expr_hinted` aceita um type hint opcional (DoD 29) para inferência
//! bidirecional top-down.

use std::cell::RefCell;
use std::collections::HashMap;

use kata_ast::{Expr, GuardClause, Pattern, Span, Spanned, TypeExpr, WithBinding};
use kata_core::dispatch::DispatchTable;
use kata_core::enum_registry::EnumRegistry;
use kata_core::escape::EscapeTarget;
use kata_core::interface_registry::InterfaceRegistry;
use kata_core::struct_registry::StructRegistry;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::RefinedDeclInfo;

use crate::typed::{TypedExpr, TypedExprKind};

use super::_match::infer_match;
use super::action_call::infer_action_call;
use super::apply::infer_apply;
use super::dot_access::infer_dot_access;
use super::helpers::InferResult;
use super::lambda::infer_lambda;
use super::sugar::{infer_pipe_fallback, infer_question};
use super::variant::resolve_unqual_variant;

/// Lambda deferido para use-site inference.
///
/// Quando `infer_lambda` não consegue resolver os tipos dos params
/// (partial dispatch ambíguo, sem hint), o lambda é guardado aqui
/// com `InferVar` nos param types. Quando a função é aplicada
/// (`f 5 3`), `infer_apply` consulta esta table e re-inere o lambda
/// com os arg types reais via `infer_apply_lambda`.
#[derive(Debug, Clone)]
pub(crate) struct DeferredLambda {
    pub patterns: Vec<Spanned<Pattern>>,
    pub body: Box<Spanned<Expr>>,
    pub guards: Vec<GuardClause>,
    pub with_bindings: Vec<WithBinding>,
    pub span: Span,
}

/// Type alias para a side table de lambdas deferidos.
pub(crate) type DeferredLambdaTable = RefCell<HashMap<String, DeferredLambda>>;

/// Contexto de inferência — carrega dependências compartilhadas entre
/// todas as funções de inferência. Substitui parâmetros individuais
/// `table` e `enum_registry`, e adiciona `ret_ty` para validação de
/// `return` em Actions.
pub(crate) struct InferCtx<'a> {
    pub table: &'a DispatchTable,
    pub enum_registry: &'a EnumRegistry,
    /// Catálogo de structs com campos — para field access e
    /// ascription-construção.
    pub struct_registry: &'a StructRegistry,
    /// Declarações refined para ascription-refined (validação
    /// compile-time de predicados sobre literais).
    pub refined_decls: &'a [RefinedDeclInfo],
    /// Catálogo de interfaces e implementações para dispatch
    /// com `iface++` no Score.
    pub interface_registry: &'a InterfaceRegistry,
    /// Catálogo de delegações `refines` — fallback no dispatch:
    /// substitui args refined pelo tipo base e retenta.
    pub refines_registry: &'a kata_core::RefinesRegistry,
    /// Tipo de retorno da Action atual — `Some(ty)` quando inferindo
    /// o body de uma Action, `None` caso contrário. Usado por `infer_return`
    /// para verificar que `return expr` produz o tipo esperado.
    pub ret_ty: Option<&'a Ty>,
    /// `true` quando inferindo dentro de um `loop`. Usado por `infer_break`
    /// e `infer_continue` para validar que só aparecem dentro de loop.
    pub in_loop: bool,
    /// Side table de lambdas deferidos para use-site inference.
    /// Quando `let f := lambda a b: - a b` falha (partial dispatch ambíguo),
    /// o lambda é guardado aqui. Quando `f 5 3` é aplicado, `infer_apply`
    /// consulta esta table e re-inere o lambda com os arg types reais.
    pub deferred_lambdas: &'a DeferredLambdaTable,
}

/// Infere o tipo de uma expressão, produzindo um `TypedExpr`.
///
/// `tail_pos` é `true` quando a expressão está em posição de cauda. O entry
/// point é sempre `tail_pos = true`. Sub-expressões de `Let` value são
/// `tail_pos = false`. Argumentos de `Apply` são `tail_pos = false`.
/// Body de lambda em tail position é `tail_pos = true`. Body de match arm
/// em tail position é `tail_pos = true`.
pub(crate) fn infer_expr(
    expr: &Expr,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    infer_expr_hinted(expr, span, env, ctx, tail_pos, None)
}

/// Verifica se `actual` cabe em `declared` — direcional (não simétrica).
/// `Var("T")` no actual significa "não-constrangido" e aceita o declarado.
/// Recursiva dentro de `Generic`.
pub(crate) fn fits_return(actual: &Ty, declared: &Ty) -> bool {
    match (actual, declared) {
        (Ty::Var(_), _) => true,
        // `return Err(e)` from `?` desugar: Result with unresolved type param
        // is a bottom/divergent expression — accept regardless of declared return type.
        (Ty::Generic(n, args), _)
            if n == "Result" && args.len() == 2 && matches!(args[0], Ty::Var(_)) =>
        {
            true
        }
        (Ty::Generic(n1, a1), Ty::Generic(n2, a2)) if n1 == n2 && a1.len() == a2.len() => {
            a1.iter().zip(a2).all(|(x, y)| fits_return(x, y))
        }
        // Tipos unários (List, Array, Range, Sender, Receiver, ReceiverFactory)
        // — recursão para o tipo interno, permitindo Var casar com concreto.
        (Ty::List(a), Ty::List(b))
        | (Ty::Array(a), Ty::Array(b))
        | (Ty::Range(a), Ty::Range(b))
        | (Ty::Sender(a), Ty::Sender(b))
        | (Ty::Receiver(a), Ty::Receiver(b))
        | (Ty::ReceiverFactory(a), Ty::ReceiverFactory(b)) => fits_return(a, b),
        _ => actual == declared,
    }
}

///
/// When `hint` is `Some(Ty::Function(params, ret))` and `expr` is a `Lambda`,
/// the params are used as the lambda's parameter types instead of InferVar.
/// When `hint` is `Some(ty)` and `expr` is a `TypeAscription`, the hint is
/// propagated to the inner expression (ascription already provides a target
/// type, so the hint is redundant there but harmless).
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_expr_hinted(
    expr: &Expr,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    let (ty, kind) = match expr {
        // ── Literais ─────────────────────────────────────────
        Expr::IntLit { text } => (Ty::int(), TypedExprKind::IntLit { text: text.clone() }),
        Expr::FloatLit { text } => (Ty::float(), TypedExprKind::FloatLit { text: text.clone() }),
        Expr::TextLit { text } => (Ty::text(), TypedExprKind::TextLit { text: text.clone() }),
        Expr::BytesLit { bytes } => (
            Ty::Bytes,
            TypedExprKind::BytesLit {
                bytes: bytes.clone(),
            },
        ),
        Expr::Unit => (Ty::Unit, TypedExprKind::Unit),

        // ── Identificador ────────────────────────────────────
        Expr::Ident { name } => {
            // Caminho 1: variável local no TypeEnv.
            if let Some(ty) = env.lookup(name).cloned() {
                (ty, TypedExprKind::Ident { name: name.clone() })
            } else {
                // Caminho 2: variante unitária desqualificada (ex: `True`,
                // `None`, `Vermelho`). Busca no EnumRegistry.
                match resolve_unqual_variant(name, span, ctx) {
                    Ok(result) => result,
                    Err(MiddleError::UnboundName {
                        name: ref err_name, ..
                    }) if !err_name.contains("ambí") && !err_name.contains("payload") => {
                        // Caminho 3: Action no DispatchTable (first-class reference).
                        // `worker` sem `!` referencia a Action como valor.
                        if let Some(overloads) = ctx.table.get_overloads(name) {
                            let action_overloads: Vec<_> =
                                overloads.iter().filter(|o| o.is_action).collect();
                            if !action_overloads.is_empty() {
                                // Primeira versão: usa o primeiro overload.
                                // TODO: overloading de Actions — resolution por tipo esperado.
                                let overload = action_overloads[0];
                                (
                                    Ty::Action(
                                        overload.params.clone(),
                                        Box::new(overload.ret.clone()),
                                    ),
                                    TypedExprKind::Ident { name: name.clone() },
                                )
                            } else {
                                // Caminho 4: realmente unbound.
                                return Err(MiddleError::UnboundName {
                                    name: name.clone(),
                                    span: (*span).into(),
                                });
                            }
                        } else {
                            return Err(MiddleError::UnboundName {
                                name: name.clone(),
                                span: (*span).into(),
                            });
                        }
                    }
                    Err(other) => return Err(other),
                }
            }
        }

        // ── Aplicação prefixa ────────────────────────────────
        // Propaga hint para infer_apply (ret-directed dispatch).
        Expr::Apply { callee, args } => infer_apply(callee, args, span, env, ctx, hint)?,

        // ── Ascription de tipo ───────────────────────────────
        Expr::TypeAscription { expr, ty } => {
            return super::ascription::infer_type_ascription(
                expr, ty, span, env, ctx, tail_pos, hint,
            );
        }

        // ── Grouping — transparente, propaga hint ────────────
        Expr::Grouping { inner } => {
            let typed_inner =
                infer_expr_hinted(&inner.node, &inner.span, env, ctx, tail_pos, hint)?;
            (
                typed_inner.ty.clone(),
                TypedExprKind::Grouping {
                    inner: Box::new(Spanned::new(typed_inner, inner.span)),
                },
            )
        }

        // ── Tuple ────────────────────────────────────────────
        Expr::Tuple { elements } => {
            let mut typed_elements = Vec::with_capacity(elements.len());
            let mut element_tys = Vec::with_capacity(elements.len());
            for elem in elements {
                let typed = infer_expr(&elem.node, &elem.span, env, ctx, false)?;
                element_tys.push(typed.ty.clone());
                typed_elements.push(Spanned::new(typed, elem.span));
            }
            (
                Ty::Tuple(element_tys),
                TypedExprKind::Tuple {
                    elements: typed_elements,
                },
            )
        }

        // ── Let binding ──────────────────────────────────────
        Expr::Let { name, value } => {
            // Tenta inferir o valor. Se falha com LambdaInferenceFail e o
            // value é um lambda, deferre para use-site inference: guarda o
            // AST do lambda na side table e define o binding com InferVars.
            // Quando `f 5 3` for aplicado, infer_apply resgata o lambda e
            // re-inere com os arg types reais.
            let typed_value = match infer_expr(&value.node, &value.span, env, ctx, false) {
                Ok(tv) => tv,
                Err(MiddleError::LambdaInferenceFail { .. })
                    if matches!(value.node, Expr::Lambda { .. }) =>
                {
                    // Deferre o lambda para use-site inference.
                    if let Expr::Lambda {
                        patterns,
                        body,
                        guards,
                        with_bindings,
                    } = &value.node
                    {
                        let n_params = patterns.len();
                        let param_types: Vec<Ty> =
                            (0..n_params).map(|i| Ty::InferVar(i as u32)).collect();
                        let ret_ty = Ty::InferVar(n_params as u32);
                        let lambda_ty = Ty::Function(param_types.clone(), Box::new(ret_ty.clone()));

                        // Guarda o AST do lambda na side table.
                        ctx.deferred_lambdas.borrow_mut().insert(
                            name.clone(),
                            DeferredLambda {
                                patterns: patterns.clone(),
                                body: body.clone(),
                                guards: guards.clone(),
                                with_bindings: with_bindings.clone(),
                                span: value.span,
                            },
                        );

                        // Constrói um TypedExpr skeleton para o lambda deferido.
                        // O codegen verá InferVar nos param_types e tratará
                        // como placeholder — o tipo real será resolvido no uso.
                        let typed_patterns = patterns
                            .iter()
                            .enumerate()
                            .map(|(i, _)| {
                                Spanned::new(
                                    crate::typed::TypedPattern::Ident {
                                        name: if let kata_ast::Pattern::Ident(n) = &patterns[i].node
                                        {
                                            n.clone()
                                        } else {
                                            format!("__param_{i}")
                                        },
                                        ty: Ty::InferVar(i as u32),
                                    },
                                    patterns[i].span,
                                )
                            })
                            .collect();
                        let skeleton_kind = TypedExprKind::Lambda {
                            func_name: None,
                            param_types: param_types.clone(),
                            ret_ty: ret_ty.clone(),
                            clauses: vec![crate::typed::TypedLambdaClause {
                                patterns: typed_patterns,
                                body: Spanned::new(
                                    TypedExpr {
                                        span: body.span,
                                        ty: ret_ty.clone(),
                                        tail_pos: true,
                                        escape: EscapeTarget::Local,
                                        kind: TypedExprKind::Unit,
                                    },
                                    body.span,
                                ),
                                guards: Vec::new(),
                                with_bindings: Vec::new(),
                            }],
                            captures: Vec::new(),
                        };
                        let skeleton = TypedExpr {
                            span: value.span,
                            ty: lambda_ty.clone(),
                            tail_pos: false,
                            escape: EscapeTarget::Local,
                            kind: skeleton_kind,
                        };

                        // Define no TypeEnv com InferVars.
                        let param_names = extract_lambda_param_names(&value.node);
                        if let Some(pnames) = param_names {
                            env.define_with_param_names(name, lambda_ty, "__local__", pnames);
                        } else {
                            env.define(name, lambda_ty, "__local__");
                        }

                        return Ok(TypedExpr {
                            span: *span,
                            ty: Ty::Unit,
                            tail_pos,
                            escape: EscapeTarget::Local,
                            kind: TypedExprKind::Let {
                                name: name.clone(),
                                value: Box::new(Spanned::new(skeleton, value.span)),
                            },
                        });
                    }
                    // Inalcançável — o guard do match garante que é Lambda.
                    return Err(MiddleError::LambdaInferenceFail {
                        span: (*span).into(),
                        detail: None,
                    });
                }
                Err(e) => return Err(e),
            };
            let val_ty = typed_value.ty.clone();

            // Rastrear provenance: se `let g := soma` onde `soma` é Ident
            // apontando para função nomeada no DispatchTable, marcar o
            // binding com `fn_alias = Some("soma")`. Isto permite que a
            // reflexão distinga alias (caso dinâmico, escalar via sidecar
            // table) de lambda com binding (caso estático, lista).
            let fn_alias = match (&value.node, &typed_value.kind) {
                (Expr::Ident { name: src_name }, _)
                    if matches!(val_ty, Ty::Function(_, _))
                        && ctx
                            .table
                            .get_overloads(src_name)
                            .is_some_and(|ols| ols.iter().any(|oi| !oi.is_action)) =>
                {
                    Some(src_name.clone())
                }
                _ => None,
            };

            // Extrair param_names se o valor é uma lambda com params nomeados.
            // Permite que o dict dispatch fallback consulte o TypeEnv quando
            // a DispatchTable não tem overloads com param_names.
            let param_names = extract_lambda_param_names(&value.node);

            if fn_alias.is_some() {
                env.define_with_alias(name, val_ty, "__local__", fn_alias);
            } else if let Some(pnames) = param_names {
                env.define_with_param_names(name, val_ty, "__local__", pnames);
            } else {
                env.define(name, val_ty, "__local__");
            }

            (
                Ty::Unit,
                TypedExprKind::Let {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
            )
        }
        // ── LetDestruct: let (x, y, ...) := expr ──────────────
        // Desugaring: `let __t := expr` + `let x := __t.0` + ...
        // Gera um único nó TAST que faz o binding temporário e os
        // bindings individuais via FieldAccess. O codegen processa
        // os Lets em sequência (cada um define uma variável local).
        Expr::LetDestruct { names, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            let val_ty = typed_value.ty.clone();
            let temp_name = "__let_destruct";

            // Define o temporário no escopo.
            env.define(temp_name, val_ty.clone(), "__local__");

            // Para cada nome (pulando `_`), define no escopo via FieldAccess.
            let mut field_bindings: Vec<(String, Spanned<TypedExpr>)> = Vec::new();
            for (i, name) in names.iter().enumerate() {
                if name == "_" {
                    continue;
                }
                // Tipo do elemento i da tupla.
                let elem_ty = match &val_ty {
                    Ty::Tuple(tys) => tys.get(i).cloned().unwrap_or(Ty::Unit),
                    _ => {
                        return Err(MiddleError::TypeMismatch {
                            expected: "tupla para destructuring".into(),
                            found: format!("{}", val_ty),
                            span: value.span.into(),
                        });
                    }
                };
                env.define(name, elem_ty.clone(), "__local__");

                // Constrói FieldAccess: __let_destruct.i
                let temp_expr = TypedExpr {
                    span: Span::synthetic(),
                    ty: val_ty.clone(),
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    kind: TypedExprKind::Ident {
                        name: temp_name.to_string(),
                    },
                };
                let access = TypedExpr {
                    span: Span::synthetic(),
                    ty: elem_ty,
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    kind: TypedExprKind::FieldAccess {
                        expr: Box::new(Spanned::new(temp_expr, Span::synthetic())),
                        struct_name: String::new(),
                        field_name: String::new(),
                        field_index: i as u32,
                    },
                };
                field_bindings.push((name.clone(), Spanned::new(access, Span::synthetic())));
            }

            (
                Ty::Unit,
                TypedExprKind::LetDestruct {
                    temp_name: temp_name.to_string(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                    bindings: field_bindings,
                },
            )
        }

        // ── Qualificação de variante (sem Apply = unitária) ─────
        // `Ident :: Ident` é ambíguo no parser: pode ser VariantQual
        // (Result::Ok) ou TypeAscription (a::Int — downcast refined→base).
        // Tentar VariantQual primeiro; se falhar (não é enum), retentar
        // como TypeAscription onde enum_name é a variável e variant é o
        // tipo alvo.
        Expr::VariantQual {
            enum_name,
            variant,
            module_path,
            ..
        } => {
            // Quando module_path qualifica (ex: core.Result::Err), resolve o
            // enum_ty do módulo de origem, não o do escopo mais próximo.
            let enum_ty = if let Some(path) = module_path.as_ref()
                && let Some(first) = path.first()
            {
                env.lookup_with_origin(enum_name, first).cloned()
            } else {
                env.lookup(enum_name).cloned()
            };

            // Caminho 1: é uma variante de enum.
            if let Some(ref ty) = enum_ty
                && let Some((vt, vk)) = super::variant_qual::infer_variant_qual(
                    enum_name,
                    variant,
                    module_path.as_deref(),
                    ty,
                    span,
                    ctx,
                )?
            {
                let escape = if ctx.ret_ty.is_some() {
                    if tail_pos {
                        EscapeTarget::Caller
                    } else {
                        EscapeTarget::Local
                    }
                } else {
                    EscapeTarget::Caller
                };
                return Ok(TypedExpr {
                    span: *span,
                    ty: vt,
                    tail_pos,
                    escape,
                    kind: vk,
                });
            }

            // Caminho 2: TypeAscription disfarçada — `var::Type`.
            // O parser produziu VariantQual porque ambos os lados são Ident.
            // Se enum_name existe no env como variável e variant é um tipo
            // conhecido, tratar como TypeAscription.
            if enum_ty.is_some() {
                let type_expr = Spanned::new(TypeExpr::Named(variant.clone()), *span);
                let inner_expr = Spanned::new(
                    Expr::Ident {
                        name: enum_name.clone(),
                    },
                    *span,
                );
                let result = super::ascription::infer_type_ascription(
                    &inner_expr,
                    &type_expr,
                    span,
                    env,
                    ctx,
                    tail_pos,
                    hint,
                )?;
                return Ok(result);
            }

            // Nem variante nem variável — unbound.
            Err(MiddleError::UnboundName {
                name: enum_name.clone(),
                span: (*span).into(),
            })?
        }

        // ── Desugared antes do typeck ──────────────────
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

        // ── Lambda ──────────────────────────────
        Expr::Lambda {
            patterns,
            body,
            guards,
            with_bindings,
        } => infer_lambda(patterns, body, guards, with_bindings, span, env, ctx, hint)?,

        // ── Match ───────────────────────────────
        Expr::Match { scrutinee, arms } => {
            infer_match(scrutinee, arms, span, env, ctx, tail_pos, hint)?
        }

        // ── ActionCall — dispatch para Action builtin ou definida ──
        Expr::ActionCall { callee, args } => {
            match infer_action_call(callee, args, span, env, ctx)? {
                super::action_call::ActionDispatch::Complete(typed) => return Ok(typed),
                super::action_call::ActionDispatch::Tuple(ty, kind) => (ty, kind),
            }
        }

        // ── Var — binding mutável (exclusivo de Actions) ──
        Expr::Var { name, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            let val_ty = typed_value.ty.clone();
            env.define_mutable(name, val_ty, "__local__");
            (
                Ty::Unit,
                TypedExprKind::Var {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
            )
        }

        // ── Reassign — reatribuição a variável `var` ──
        Expr::Reassign { name, value } => {
            // Verifica que a variável existe e foi declarada como mutável.
            let existing_ty =
                env.lookup(name)
                    .cloned()
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: name.clone(),
                        span: (*span).into(),
                    })?;
            if !env.is_mutable(name) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("variável mutável `{name}` (declarada com `var`)"),
                    found: format!("variável imutável `{name}` (declarada com `let`)"),
                    span: (*span).into(),
                });
            }
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            if typed_value.ty != existing_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{existing_ty:?}"),
                    found: format!("{}", typed_value.ty),
                    span: value.span.into(),
                });
            }
            (
                Ty::Unit,
                TypedExprKind::Reassign {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
            )
        }

        // ── Return — early return de Action ──
        Expr::Return(inner) => {
            let ret_ty = ctx.ret_ty.ok_or_else(|| MiddleError::TypeMismatch {
                expected: "return dentro de Action".into(),
                found: "return fora de Action".into(),
                span: (*span).into(),
            })?;
            let expanded_ret = ctx.enum_registry.expand_defaults(ret_ty);
            let typed_inner = infer_expr(&inner.node, &inner.span, env, ctx, false)?;
            if !fits_return(&typed_inner.ty, &expanded_ret) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{ret_ty:?}"),
                    found: format!("{}", typed_inner.ty),
                    span: inner.span.into(),
                });
            }
            (
                typed_inner.ty.clone(),
                TypedExprKind::Return(Box::new(Spanned::new(typed_inner, inner.span))),
            )
        }
        Expr::Loop { body } => {
            // Loop body é inferido com in_loop = true.
            // Cada expr do body é inferida em sequência no mesmo escopo.
            // O tipo do loop é Unit (break sem valor).
            let loop_ctx = InferCtx {
                table: ctx.table,
                enum_registry: ctx.enum_registry,
                struct_registry: ctx.struct_registry,
                refined_decls: ctx.refined_decls,
                interface_registry: ctx.interface_registry,
                refines_registry: ctx.refines_registry,
                ret_ty: ctx.ret_ty,
                in_loop: true,
                deferred_lambdas: ctx.deferred_lambdas,
            };
            let mut typed_body = Vec::new();
            for expr in body {
                let typed = infer_expr(
                    &expr.node, &expr.span, env, &loop_ctx,
                    false, // body do loop nunca é tail_pos (loop retorna Unit)
                )?;
                typed_body.push(Spanned::new(typed, expr.span));
            }
            (Ty::Unit, TypedExprKind::Loop { body: typed_body })
        }
        Expr::Break => {
            if !ctx.in_loop {
                return Err(MiddleError::TypeMismatch {
                    expected: "expressão (break só existe dentro de loop)".into(),
                    found: "Break".into(),
                    span: (*span).into(),
                });
            }
            (Ty::Unit, TypedExprKind::Break)
        }
        Expr::Continue => {
            if !ctx.in_loop {
                return Err(MiddleError::TypeMismatch {
                    expected: "expressão (continue só existe dentro de loop)".into(),
                    found: "Continue".into(),
                    span: (*span).into(),
                });
            }
            (Ty::Unit, TypedExprKind::Continue)
        }

        // ── `?` fail-fast — desugar para Match + Return ──
        Expr::Question(inner) => {
            return infer_question(inner, span, env, ctx, tail_pos);
        }
        // ── `|` fallback — desugar para Match (coalescência pura) ──
        Expr::PipeFallback { lhs, rhs } => {
            return infer_pipe_fallback(lhs, rhs, span, env, ctx);
        }
        // ── DotAccess (field access + index access) ──
        Expr::DotAccess { expr, index } => {
            return infer_dot_access(expr, index, span, env, ctx, tail_pos);
        }
        // ── Spread ($) — typeck expande, nunca deveria chegar aqui ──
        Expr::Spread => {
            return Err(MiddleError::UnboundName {
                name: "Spread ($) em posição inesperada — typeck deveria ter expandido".into(),
                span: (*span).into(),
            });
        }
        // ── Coleções — inferência delegada para collections.rs ──
        Expr::ListLit { elements } => {
            return super::collections::infer_list_lit(elements, span, env, ctx, tail_pos, hint);
        }
        Expr::ArrayLit { elements } => {
            return super::collections::infer_array_lit(elements, span, env, ctx, tail_pos);
        }
        Expr::DictLit { entries } => {
            return super::dict_set::infer_dict_lit(entries, span, env, ctx, tail_pos);
        }
        Expr::SetLit { elements } => {
            return super::dict_set::infer_set_lit(elements, span, env, ctx, tail_pos);
        }
        Expr::RangeLit {
            start,
            step,
            end,
            inclusive,
        } => {
            return super::collections::infer_range_lit(
                start, step, end, *inclusive, span, env, ctx, tail_pos,
            );
        }
        // ── ForIn e In ───────────────────────────────────────
        Expr::ForIn {
            var_name,
            iterable,
            body,
        } => {
            return super::collections::infer_for_in(
                var_name, iterable, body, span, env, ctx, tail_pos,
            );
        }
        Expr::In { item, collection } => {
            return super::collections::infer_in(item, collection, span, env, ctx, tail_pos);
        }

        // ── CSP — typeck em csp.rs ──
        Expr::ChannelSend { channel, value } => {
            return super::csp::infer_channel_send(channel, value, span, env, ctx, tail_pos);
        }
        Expr::ChannelRecv { channel, bind_name } => {
            return super::csp::infer_channel_recv(channel, bind_name, span, env, ctx, tail_pos);
        }
        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            return super::csp::infer_select(
                arms,
                timeout_ms,
                timeout_body,
                span,
                env,
                ctx,
                tail_pos,
            );
        }

        // ── `type!(expr)` — introspecção compile-time ──
        // ty = Text (sempre), kind = TypeOf { expr: typed_inner }.
        Expr::TypeOf { expr: inner } => {
            let typed_inner = infer_expr(&inner.node, &inner.span, env, ctx, false)?;
            (
                Ty::text(),
                TypedExprKind::TypeOf {
                    expr: Box::new(Spanned::new(typed_inner, inner.span)),
                },
            )
        }

        // ── `@comptime expr` — marca para avaliação em compile-time ──
        // O typeck tipa o inner expr normalmente. O comptime pass (que corre
        // depois do tree shaking) identifica estes nós, verifica constness +
        // pureza, JIT-executa, e substitui por Literal/HeapSnapshot.
        // O tipo é o mesmo do inner expr — comptime não muda o tipo.
        Expr::Comptime { expr: inner } => {
            let typed_inner = infer_expr(&inner.node, &inner.span, env, ctx, tail_pos)?;
            let inner_ty = typed_inner.ty.clone();
            (
                inner_ty,
                TypedExprKind::Comptime {
                    expr: Box::new(Spanned::new(typed_inner, inner.span)),
                },
            )
        }
        // ── Block: sequência de expressões —──────────────────
        // Usado em match arm body indentado com múltiplas statements.
        // Cada statement é inferida em sequência no mesmo escopo.
        // O resultado é a última expressão.
        Expr::Block { stmts } => {
            let mut typed_stmts: Vec<Spanned<TypedExpr>> = Vec::new();
            let mut last_ty = Ty::Unit;
            for stmt in stmts {
                let typed_stmt = infer_expr(&stmt.node, &stmt.span, env, ctx, tail_pos)?;
                last_ty = typed_stmt.ty.clone();
                typed_stmts.push(Spanned::new(typed_stmt, stmt.span));
            }
            (last_ty, TypedExprKind::Block { stmts: typed_stmts })
        }
    };

    // Deriva EscapeTarget de tail_pos + contexto (Action vs função pura/entry).
    let escape = if ctx.ret_ty.is_some() {
        // Dentro de Action: tail_pos = true → Caller, false → Local.
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        // Função pura / entry point: sem fiber_arena, tudo vai para a raiz.
        EscapeTarget::Caller
    };

    Ok(TypedExpr {
        span: *span,
        ty,
        tail_pos,
        escape,
        kind,
    })
}

/// Extrai nomes dos parâmetros de uma lambda, se todos são nomeados (Ident ou TypedIdent).
/// Retorna `None` se a expressão não é lambda ou se algum pattern não tem nome
/// (Wildcard, Tuple, Cons, etc.).
fn extract_lambda_param_names(expr: &Expr) -> Option<Vec<String>> {
    if let Expr::Lambda { patterns, .. } = expr {
        let mut names = Vec::with_capacity(patterns.len());
        for p in patterns {
            match &p.node {
                kata_ast::Pattern::Ident(name) => names.push(name.clone()),
                kata_ast::Pattern::TypedIdent { name, .. } => names.push(name.clone()),
                _ => return None, // Wildcard, Tuple, Cons, etc. — não nomeado
            }
        }
        Some(names)
    } else {
        None
    }
}
