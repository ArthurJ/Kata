//! Log builtins — `log!`, `log_recv!`, `log_config!`.
//!
//! Extraído de `action_call.rs` — os três builtins de log são self-contained:
//! compartilham apenas `extract_tuple_elements` e `resolve_log_level`
//! (ambos exclusivos deste módulo), não dependem dos builtins CSP e não
//! chamam `infer_channel_builtin`/`infer_queue_builtin`/`infer_fork_builtin`.

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::action_call::ActionDispatch;
use super::expr::InferCtx;
use super::helpers::InferResult;

/// `log!(level, msg, topic?, policy?)` — desugara para `kata_rt_log_publish`.
///
/// Args posicionais:
/// - 0: LogLevel (VariantQual ex: `LogLevel::Info` → tag i64)
/// - 1: Text (mensagem dinâmica)
/// - 2: Text (tópico, opcional → 0 = config herdada)
/// - 3: Text (policy, opcional → 0 = config herdada)
pub(crate) fn infer_log_builtin(
    args: &Spanned<Expr>,
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    let elements = extract_tuple_elements(args)?;
    if elements.len() < 2 || elements.len() > 4 {
        return Err(MiddleError::ArityMismatch {
            expected: 4, // aceita 2-4, mas informamos max
            found: elements.len(),
            span: args.span.into(),
        hint: None,
        });
    }

    // Level: VariantQual (LogLevel::Info) → tag i64 via enum_registry.
    let level_typed =
        super::expr::infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
    let level_val = resolve_log_level(&level_typed, ctx, &elements[0].span)?;

    // Msg: Text.
    let msg_typed = super::expr::infer_expr(&elements[1].node, &elements[1].span, env, ctx, false)?;
    if msg_typed.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", msg_typed.ty),
            span: elements[1].span.into(),
        });
    }

    // Topic: Text opcional → 0 se ausente (Unit lowera como iconst(0), não SMI).
    let topic_typed = if let Some(elem) = elements.get(2) {
        let t = super::expr::infer_expr(&elem.node, &elem.span, env, ctx, false)?;
        if t.ty != Ty::text() {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{}", Ty::text()),
                found: format!("{}", t.ty),
                span: elem.span.into(),
            });
        }
        t
    } else {
        TypedExpr {
            span: args.span,
            ty: Ty::int(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::Unit,
        }
    };

    // Policy: Text opcional → 0 se ausente (Unit lowera como iconst(0), não SMI).
    let policy_typed = if let Some(elem) = elements.get(3) {
        let t = super::expr::infer_expr(&elem.node, &elem.span, env, ctx, false)?;
        if t.ty != Ty::text() {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{}", Ty::text()),
                found: format!("{}", t.ty),
                span: elem.span.into(),
            });
        }
        t
    } else {
        TypedExpr {
            span: args.span,
            ty: Ty::int(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::Unit,
        }
    };

    // Constrói Closure { ffi_symbol: "kata_rt_log_publish" }.
    let callee = TypedExpr {
        span: args.span,
        ty: Ty::Function(
            vec![Ty::int(), Ty::text(), Ty::text(), Ty::text()],
            Box::new(Ty::int()),
        ),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_publish".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: Ty::int(),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Caller,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, args.span)),
            // Ordem dos args coincide com a assinatura da FFI:
            // kata_rt_log_publish(topic_ptr, level, msg, policy_ptr).
            args: vec![
                Spanned::new(
                    topic_typed,
                    elements.get(2).map(|e| e.span).unwrap_or(args.span),
                ),
                Spanned::new(level_val, elements[0].span),
                Spanned::new(msg_typed, elements[1].span),
                Spanned::new(
                    policy_typed,
                    elements.get(3).map(|e| e.span).unwrap_or(args.span),
                ),
            ],
            ffi_symbol: Some("kata_rt_log_publish".into()),
        },
    };

    Ok(ActionDispatch::Complete(typed))
}

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

    let callee = TypedExpr {
        span: args.span,
        ty: Ty::Function(vec![Ty::text()], Box::new(Ty::text())),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_recv".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: Ty::text(),
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
    let level_val = resolve_log_level(&level_typed, ctx, &elements[2].span)?;

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

/// Resolve uma expressão LogLevel (VariantQual ou IntLit) para tag i64.
fn resolve_log_level(
    typed: &TypedExpr,
    _ctx: &InferCtx,
    span: &Span,
) -> Result<TypedExpr, MiddleError> {
    // Se já é IntLit, retorna.
    if let TypedExprKind::IntLit { .. } = &typed.kind {
        return Ok(typed.clone());
    }

    // Se é VariantQual de LogLevel, extrai a tag.
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
