//! Núcleo da inferência de expressões — o grande match sobre `Expr`.
//!
//! `infer_expr` é o entry point público (usado por todos os submódulos).
//! `infer_expr_hinted` aceita um type hint opcional (DoD 29) para inferência
//! bidirecional top-down.

use kata_ast::{Expr, Span, Spanned};
use kata_core::dispatch::DispatchTable;
use kata_core::enum_registry::EnumRegistry;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::_match::infer_match;
use super::apply::infer_apply;
use super::helpers::{InferResult, resolve_type_expr};
use super::lambda::infer_lambda;
use super::sugar::{infer_assert, infer_pipe_fallback, infer_question};

/// Contexto de inferência — carrega dependências compartilhadas entre
/// todas as funções de inferência. Substitui parâmetros individuais
/// `table` e `enum_registry`, e adiciona `ret_ty` para validação de
/// `return` em Actions (Fase 2).
pub(crate) struct InferCtx<'a> {
    pub table: &'a DispatchTable,
    pub enum_registry: &'a EnumRegistry,
    /// Tipo de retorno da Action atual — `Some(ty)` quando inferindo
    /// o body de uma Action, `None` caso contrário. Usado por `infer_return`
    /// para verificar que `return expr` produz o tipo esperado.
    pub ret_ty: Option<&'a Ty>,
    /// `true` quando inferindo dentro de um `loop`. Usado por `infer_break`
    /// e `infer_continue` para validar que só aparecem dentro de loop.
    pub in_loop: bool,
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
        (Ty::Generic(n1, a1), Ty::Generic(n2, a2)) if n1 == n2 && a1.len() == a2.len() => {
            a1.iter().zip(a2).all(|(x, y)| fits_return(x, y))
        }
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
            // Caminho normal: é uma binding no type_env.
            if let Some(ty) = env.lookup(name).cloned() {
                (
                    ty,
                    TypedExprKind::Ident { name: name.clone() },
                    Effect::Puro,
                )
            } else {
                // Fallback: variante unitária desqualificada (ex: `True`,
                // `None`, `Vermelho`). Busca no EnumRegistry.
                resolve_unqual_variant(name, span, ctx)?
            }
        }

        // ── Aplicação prefixa ────────────────────────────────
        Expr::Apply { callee, args } => infer_apply(callee, args, span, env, ctx)?,

        // ── Ascription de tipo ───────────────────────────────
        Expr::TypeAscription { expr, ty } => {
            let target_ty = resolve_type_expr(&ty.node, env);
            // Propaga o tipo anotado como hint top-down (DoD 29).
            // Isto permite que `(lambda x: + x 1)::(Int -> Int)` extraia
            // x: Int do tipo anotado.
            let inner =
                infer_expr_hinted(&expr.node, &expr.span, env, ctx, false, Some(&target_ty))?;

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

        // ── Grouping — transparente, propaga hint ────────────
        Expr::Grouping { inner } => {
            let typed_inner =
                infer_expr_hinted(&inner.node, &inner.span, env, ctx, tail_pos, hint)?;
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
                let typed = infer_expr(&elem.node, &elem.span, env, ctx, false)?;
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
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
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

        // ── Qualificação de variante (sem Apply = unitária) ─────
        Expr::VariantQual { enum_name, variant } => {
            let enum_ty =
                env.lookup(enum_name)
                    .cloned()
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: enum_name.clone(),
                        span: (*span).into(),
                    })?;

            match &enum_ty {
                // Fase 6: enum genérico no TypeEnv como Ty::Sum, mas o EnumRegistry
                // marca como genérico. Para variantes unitárias (Optional::None),
                // produz Ty::Generic com type_args não-inferidos (Ty::Var).
                Ty::Sum(name) if ctx.enum_registry.is_generic(name) => {
                    if !ctx.enum_registry.is_variant(name, variant) {
                        return Err(MiddleError::UnboundName {
                            name: format!("{}::{}", name, variant),
                            span: (*span).into(),
                        });
                    }
                    if ctx.enum_registry.payload_ty(name, variant).is_some() {
                        return Err(MiddleError::TypeMismatch {
                            expected: "aplicação de argumento (Result::Ok valor)".into(),
                            found: format!("{}::{} tem payload — use Apply", name, variant),
                            span: (*span).into(),
                        });
                    }
                    let tag = ctx
                        .enum_registry
                        .variant_index(name, variant)
                        .ok_or_else(|| MiddleError::UnboundName {
                            name: format!("{}::{}", name, variant),
                            span: (*span).into(),
                        })?;
                    // Para variantes unitárias de enum genérico (Optional::None),
                    // não há arg para inferir os type params. Produz Ty::Generic
                    // com type_args como Ty::Var (não-inferido).
                    let type_params = ctx
                        .enum_registry
                        .type_params_of(name)
                        .expect("is_generic true");
                    let type_args: Vec<Ty> =
                        type_params.iter().map(|p| Ty::Var(p.clone())).collect();
                    let result_ty = Ty::Generic(name.clone(), type_args);
                    (
                        result_ty,
                        TypedExprKind::VariantQual {
                            enum_name: name.clone(),
                            variant: variant.clone(),
                            tag,
                        },
                        Effect::Puro,
                    )
                }
                Ty::Sum(name) => {
                    // Verifica que a variante existe.
                    if !ctx.enum_registry.is_variant(name, variant) {
                        return Err(MiddleError::UnboundName {
                            name: format!("{}::{}", name, variant),
                            span: (*span).into(),
                        });
                    }
                    // Fase 5: VariantQual sem Apply só é válido para variantes unitárias.
                    // Variantes com payload exigem Apply (Result::Ok 42).
                    if ctx.enum_registry.payload_ty(name, variant).is_some() {
                        return Err(MiddleError::TypeMismatch {
                            expected: "aplicação de argumento (Result::Ok valor)".into(),
                            found: format!("{}::{} tem payload — use Apply", name, variant),
                            span: (*span).into(),
                        });
                    }
                    let tag = ctx
                        .enum_registry
                        .variant_index(name, variant)
                        .ok_or_else(|| MiddleError::UnboundName {
                            name: format!("{}::{}", name, variant),
                            span: (*span).into(),
                        })?;
                    (
                        enum_ty.clone(),
                        TypedExprKind::VariantQual {
                            enum_name: name.clone(),
                            variant: variant.clone(),
                            tag,
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
        } => infer_lambda(patterns, body, guards, with_bindings, span, env, ctx, hint)?,

        // ── Fio 2 Fase 8: Match ───────────────────────────────
        Expr::Match { scrutinee, arms } => infer_match(scrutinee, arms, span, env, ctx, tail_pos)?,

        // ── Fio 3: ActionCall — dispatch para Action builtin ou definida ──
        Expr::ActionCall { callee, args } => {
            // Fase 9: assert! é desugared no typeck para
            // match cond { True: Unit, False: panic!(msg) }.
            if callee == "assert" {
                return infer_assert(args, span, env, ctx);
            }

            // Lowera a tupla de argumentos.
            let typed_args = infer_expr(&args.node, &args.span, env, ctx, false)?;

            // Normaliza Grouping → Tuple de 1 elemento para ActionCall args.
            // `action!(x)` produz Grouping no parser; o codegen precisa de Tuple
            // (ponteiro para array na arena) para passar args_ptr corretamente.
            let typed_args = match &typed_args.kind {
                TypedExprKind::Grouping { inner } => {
                    let inner = inner.clone();
                    TypedExpr {
                        ty: Ty::Tuple(vec![inner.node.ty.clone()]),
                        kind: TypedExprKind::Tuple {
                            elements: vec![*inner],
                        },
                        span: typed_args.span,
                        tail_pos: typed_args.tail_pos,
                        effect: typed_args.effect,
                    }
                }
                _ => typed_args,
            };

            // Extrai tipos dos elementos da tupla para dispatch.
            let arg_tys: Vec<Ty> = match &typed_args.kind {
                TypedExprKind::Tuple { elements } => {
                    elements.iter().map(|e| e.node.ty.clone()).collect()
                }
                TypedExprKind::Unit => Vec::new(), // `!()` = tupla vazia
                _ => vec![typed_args.ty.clone()],  // args não-tupla (não deveria acontecer)
            };

            // Resolve no DispatchTable.
            let overload = ctx
                .table
                .resolve(callee, &arg_tys)
                .map_err(|e| super::helpers::dispatch_to_middle_error(e, *span))?;

            // Verifica que é uma Action (is_action = true).
            if !overload.is_action {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("Action `{callee}` (is_action=true)"),
                    found: format!("função pura `{callee}` — use sem `!`"),
                    span: (*span).into(),
                });
            }

            (
                overload.ret,
                TypedExprKind::ActionCall {
                    callee: callee.clone(),
                    args: Box::new(Spanned::new(typed_args, args.span)),
                    caller_arena: 0, // placeholder — preenchido no codegen
                    ffi_symbol: overload.ffi_symbol.clone().filter(|_s| overload.is_action),
                },
                Effect::Puro, // Fio 3 não ativa Effect
            )
        }

        // ── Fio 3: var — binding mutável (exclusivo de Actions) ──
        Expr::Var { name, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            let val_ty = typed_value.ty.clone();
            env.define_mutable(name, val_ty);
            (
                Ty::Unit,
                TypedExprKind::Var {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
                Effect::Puro,
            )
        }

        // ── Fio 3: Reassign — reatribuição a variável `var` ──
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
                    found: format!("{:?}", typed_value.ty),
                    span: value.span.into(),
                });
            }
            (
                Ty::Unit,
                TypedExprKind::Reassign {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
                Effect::Puro,
            )
        }

        // ── Fio 3: return — early return de Action (Fase 2) ──
        Expr::Return(inner) => {
            let ret_ty = ctx.ret_ty.ok_or_else(|| MiddleError::TypeMismatch {
                expected: "return dentro de Action".into(),
                found: "return fora de Action".into(),
                span: (*span).into(),
            })?;
            let typed_inner = infer_expr(&inner.node, &inner.span, env, ctx, false)?;
            if !fits_return(&typed_inner.ty, ret_ty) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{ret_ty:?}"),
                    found: format!("{:?}", typed_inner.ty),
                    span: inner.span.into(),
                });
            }
            (
                typed_inner.ty.clone(),
                TypedExprKind::Return(Box::new(Spanned::new(typed_inner, inner.span))),
                Effect::Puro,
            )
        }
        Expr::Loop { body } => {
            // Loop body é inferido com in_loop = true.
            // Cada expr do body é inferida em sequência no mesmo escopo.
            // O tipo do loop é Unit (break sem valor na Fase 4).
            let loop_ctx = InferCtx {
                table: ctx.table,
                enum_registry: ctx.enum_registry,
                ret_ty: ctx.ret_ty,
                in_loop: true,
            };
            let mut typed_body = Vec::new();
            for expr in body {
                let typed = infer_expr(
                    &expr.node, &expr.span, env, &loop_ctx,
                    false, // body do loop nunca é tail_pos (loop retorna Unit)
                )?;
                typed_body.push(Spanned::new(typed, expr.span));
            }
            (
                Ty::Unit,
                TypedExprKind::Loop { body: typed_body },
                Effect::Puro,
            )
        }
        Expr::Break => {
            if !ctx.in_loop {
                return Err(MiddleError::TypeMismatch {
                    expected: "expressão (break só existe dentro de loop)".into(),
                    found: "Break".into(),
                    span: (*span).into(),
                });
            }
            (Ty::Unit, TypedExprKind::Break, Effect::Puro)
        }
        Expr::Continue => {
            if !ctx.in_loop {
                return Err(MiddleError::TypeMismatch {
                    expected: "expressão (continue só existe dentro de loop)".into(),
                    found: "Continue".into(),
                    span: (*span).into(),
                });
            }
            (Ty::Unit, TypedExprKind::Continue, Effect::Puro)
        }

        // ── Fase 7: `?` fail-fast — desugar para Match + Return ──
        Expr::Question(inner) => {
            return infer_question(inner, span, env, ctx, tail_pos);
        }
        // ── Fase 8: `|` fallback — desugar para Match (coalescência pura) ──
        Expr::PipeFallback { lhs, rhs } => {
            return infer_pipe_fallback(lhs, rhs, span, env, ctx);
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

/// Resolve variante desqualificada em posição de expressão.
///
/// Quando `env.lookup(name)` falha, tenta o EnumRegistry: se `name` é variante
/// unitária de exatamente 1 enum, produz `VariantQual`. Se múltiplos enums têm
/// a variante, erro de ambiguidade. Se 0, `UnboundName`. Se tem payload, erro
/// (precisa de Apply: `Ok 42`, não `Ok` sozinho).
fn resolve_unqual_variant(
    name: &str,
    span: &Span,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    let candidates = ctx.enum_registry.find_enums_with_variant(name);
    if candidates.is_empty() {
        return Err(MiddleError::UnboundName {
            name: name.to_string(),
            span: (*span).into(),
        });
    }
    if candidates.len() > 1 {
        return Err(MiddleError::UnboundName {
            name: format!(
                "variante '{name}' é ambígua — existe em: {}. Qualifique (ex: {}::{name})",
                candidates.join(", "),
                candidates[0]
            ),
            span: (*span).into(),
        });
    }
    let enum_name = candidates[0];
    if ctx.enum_registry.payload_ty(enum_name, name).is_some() {
        return Err(MiddleError::UnboundName {
            name: format!(
                "{enum_name}::{name} tem payload — use Apply (ex: {enum_name}::{name} valor)"
            ),
            span: (*span).into(),
        });
    }
    let tag = ctx.enum_registry.variant_index(enum_name, name).unwrap_or(0);
    if ctx.enum_registry.is_generic(enum_name) {
        let type_params = ctx
            .enum_registry
            .type_params_of(enum_name)
            .expect("is_generic true");
        let type_args: Vec<Ty> = type_params.iter().map(|p| Ty::Var(p.clone())).collect();
        Ok((
            Ty::Generic(enum_name.to_string(), type_args),
            TypedExprKind::VariantQual {
                enum_name: enum_name.to_string(),
                variant: name.to_string(),
                tag,
            },
            Effect::Puro,
        ))
    } else {
        Ok((
            Ty::Sum(enum_name.to_string()),
            TypedExprKind::VariantQual {
                enum_name: enum_name.to_string(),
                variant: name.to_string(),
                tag,
            },
            Effect::Puro,
        ))
    }
}
