//! Log builtins — `log_recv!`, `log_config!`.
//!
//! `log!()` foi migrado para action normal do stdlib (overloads no DispatchTable).
//! Restam aqui apenas as duas interceptadas que desugam para FFI direta:
//! `log_recv!()` e `log_config!()`.
//!
//! `resolve_log_level` é usado por `log_config!()` — aceita VariantQual
//! (compile-time) e Int (fallback).

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::action_call::ActionDispatch;
use super::expr::InferCtx;
use super::helpers::InferResult;

/// `log_recv!(topic)` — desugara para `kata_rt_log_recv`.
pub(crate) fn infer_log_recv_builtin(
    args: &Spanned<Expr>,
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    let elements = extract_tuple_elements(args)?;
    if elements.len() != 1 {
        return Err(MiddleError::ArityMismatch {
            expected: 1,
            found: elements.len(),
            span: args.span.into(),
            hint: None,
        });
    }

    let topic_typed =
        super::expr::infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
    if topic_typed.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", topic_typed.ty),
            span: elements[0].span.into(),
        });
    }

    let result_ty = Ty::Generic("Result".into(), vec![Ty::text(), Ty::text()]);

    let callee = TypedExpr {
        span: args.span,
        ty: Ty::Function(vec![Ty::text()], Box::new(result_ty.clone())),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_recv".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: result_ty,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Caller,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, args.span)),
            args: vec![Spanned::new(topic_typed, elements[0].span)],
            ffi_symbol: Some("kata_rt_log_recv".into()),
        },
    };

    Ok(ActionDispatch::Complete(typed))
}

/// `log_config!(topic, policy, level)` — desugara para `kata_rt_log_config`.
pub(crate) fn infer_log_config_builtin(
    args: &Spanned<Expr>,
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    let elements = extract_tuple_elements(args)?;
    if elements.len() != 3 {
        return Err(MiddleError::ArityMismatch {
            expected: 3,
            found: elements.len(),
            span: args.span.into(),
            hint: None,
        });
    }

    let topic_typed =
        super::expr::infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
    if topic_typed.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", topic_typed.ty),
            span: elements[0].span.into(),
        });
    }

    let policy_typed =
        super::expr::infer_expr(&elements[1].node, &elements[1].span, env, ctx, false)?;
    if policy_typed.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", policy_typed.ty),
            span: elements[1].span.into(),
        });
    }

    let level_typed =
        super::expr::infer_expr(&elements[2].node, &elements[2].span, env, ctx, false)?;
    let level_val = resolve_log_level(&level_typed, &elements[2].span)?;

    let callee = TypedExpr {
        span: args.span,
        ty: Ty::Function(vec![Ty::text(), Ty::text(), Ty::int()], Box::new(Ty::Unit)),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_config".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: Ty::Unit,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Caller,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, args.span)),
            args: vec![
                Spanned::new(topic_typed, elements[0].span),
                Spanned::new(policy_typed, elements[1].span),
                Spanned::new(level_val, elements[2].span),
            ],
            ffi_symbol: Some("kata_rt_log_config".into()),
        },
    };

    Ok(ActionDispatch::Complete(typed))
}

/// Extrai elementos de uma tupla, Grouping de tupla, ou Grouping de expr única.
///
/// O parser produz `Grouping(inner)` para `f!(x)` (1 arg) e `Tuple([...])` para
/// `f!(x, y)` (2+ args). Para builtins como `log_recv!(x)` que exigem 1 arg,
/// tratamos `Grouping(x)` como tupla de 1 elemento `[x]` — mesmo padrão que
/// `infer_action_call` aplica ao normalizar args de ActionCall.
fn extract_tuple_elements(args: &Spanned<Expr>) -> Result<Vec<Spanned<Expr>>, MiddleError> {
    match &args.node {
        Expr::Tuple { elements } => Ok(elements.clone()),
        Expr::Grouping { inner } => match &inner.node {
            Expr::Tuple { elements } => Ok(elements.clone()),
            // Grouping de expr única = tupla de 1 elemento.
            // `log_recv!("x")` parseia como Grouping(TextLit("x")).
            other => Ok(vec![Spanned::new(other.clone(), inner.span)]),
        },
        Expr::Unit => Ok(Vec::new()),
        _ => Err(MiddleError::TypeMismatch {
            expected: "Tuple".into(),
            found: format!("{:?}", args.node),
            span: args.span.into(),
        }),
    }
}

/// Resolve uma expressão LogLevel (VariantQual) para tag i64.
///
/// Usado por `log_config!()`. Aceita:
/// - `VariantQual` de LogLevel (ex: `LogLevel::Warn`) → tag em compile-time
/// - `VariantConstruct` de LogLevel → tag em compile-time
/// - `IntLit` → direto
/// - `Int` → direto (fallback)
fn resolve_log_level(
    typed: &TypedExpr,
    span: &kata_ast::Span,
) -> Result<TypedExpr, MiddleError> {
    // Se já é IntLit, retorna.
    if let TypedExprKind::IntLit { .. } = &typed.kind {
        return Ok(typed.clone());
    }

    // Se é VariantQual de LogLevel, extrai a tag em compile-time.
    if let TypedExprKind::VariantQual {
        enum_name,
        variant: _,
        tag,
        ..
    } = &typed.kind
        && enum_name == "LogLevel"
    {
        return Ok(TypedExpr {
            span: typed.span,
            ty: Ty::int(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::IntLit {
                text: tag.to_string(),
            },
        });
    }

    // Se é VariantConstruct de LogLevel.
    if let TypedExprKind::VariantConstruct {
        enum_name,
        variant: _,
        tag,
        ..
    } = &typed.kind
        && enum_name == "LogLevel"
    {
        return Ok(TypedExpr {
            span: typed.span,
            ty: Ty::int(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::IntLit {
                text: tag.to_string(),
            },
        });
    }

    // Fallback: se o tipo é Int, usa direto.
    if typed.ty == Ty::int() {
        return Ok(typed.clone());
    }

    Err(MiddleError::TypeMismatch {
        expected: "LogLevel variant (Debug, Info, Warn, Error) ou Int".into(),
        found: format!("{}", typed.ty),
        span: (*span).into(),
    })
}