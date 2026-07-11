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
    table: &DispatchTable,
    enum_registry: &EnumRegistry,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    infer_expr_hinted(expr, span, env, table, enum_registry, tail_pos, None)
}

/// Like `infer_expr` but accepts an optional type hint (DoD 29).
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
    table: &DispatchTable,
    enum_registry: &EnumRegistry,
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
        Expr::Apply { callee, args } => infer_apply(callee, args, span, env, table, enum_registry)?,

        // ── Ascription de tipo ───────────────────────────────
        Expr::TypeAscription { expr, ty } => {
            let target_ty = resolve_type_expr(&ty.node, env);
            // Propaga o tipo anotado como hint top-down (DoD 29).
            // Isto permite que `(lambda x: + x 1)::(Int -> Int)` extraia
            // x: Int do tipo anotado.
            let inner = infer_expr_hinted(
                &expr.node,
                &expr.span,
                env,
                table,
                enum_registry,
                false,
                Some(&target_ty),
            )?;

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
            let typed_inner = infer_expr_hinted(
                &inner.node,
                &inner.span,
                env,
                table,
                enum_registry,
                tail_pos,
                hint,
            )?;
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
            let typed_value =
                infer_expr(&value.node, &value.span, env, table, enum_registry, false)?;
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
            let enum_ty =
                env.lookup(enum_name)
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
            hint,
        )?,

        // ── Fio 2 Fase 8: Match ───────────────────────────────
        Expr::Match { scrutinee, arms } => {
            infer_match(scrutinee, arms, span, env, table, enum_registry, tail_pos)?
        }

        // ── Fio 3: ActionCall — dispatch para Action builtin ou definida ──
        Expr::ActionCall { callee, args } => {
            // Lowera a tupla de argumentos.
            let typed_args = infer_expr(&args.node, &args.span, env, table, enum_registry, false)?;
            // Extrai tipos dos elementos da tupla para dispatch.
            let arg_tys: Vec<Ty> = match &typed_args.kind {
                TypedExprKind::Tuple { elements } => {
                    elements.iter().map(|e| e.node.ty.clone()).collect()
                }
                TypedExprKind::Unit => Vec::new(), // `!()` = tupla vazia
                _ => vec![typed_args.ty.clone()],  // args não-tupla (não deveria acontecer)
            };

            // Resolve no DispatchTable.
            let overload = table
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
            let typed_value =
                infer_expr(&value.node, &value.span, env, table, enum_registry, false)?;
            let val_ty = typed_value.ty.clone();
            env.define(name, val_ty);
            (
                Ty::Unit,
                TypedExprKind::Var {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
                Effect::Puro,
            )
        }

        // ── Fio 3: return/loop/break/continue — Fase 2 e 4 ──
        Expr::Return(_) => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (return em Actions — Fase 2)".into(),
                found: "Return".into(),
                span: (*span).into(),
            });
        }
        Expr::Loop { .. } => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (loop em Actions — Fase 4)".into(),
                found: "Loop".into(),
                span: (*span).into(),
            });
        }
        Expr::Break => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (break em Actions — Fase 4)".into(),
                found: "Break".into(),
                span: (*span).into(),
            });
        }
        Expr::Continue => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (continue em Actions — Fase 4)".into(),
                found: "Continue".into(),
                span: (*span).into(),
            });
        }

        // ── Fio 3: Question e PipeFallback — desugar é Fase 7 e 8 ──
        Expr::Question(_) => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (? desugar — Fase 7)".into(),
                found: "Question".into(),
                span: (*span).into(),
            });
        }
        Expr::PipeFallback { .. } => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (| desugar — Fase 8)".into(),
                found: "PipeFallback".into(),
                span: (*span).into(),
            });
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
