//! Builtins CSP (Communicating Sequential Processes) — channel, queue, fork.
//!
//! Extraído de `action_call.rs` — três funções self-contained que lidam com
//! os builtins de concorrência:
//! - `channel!()` / `broadcast!()` → `ChannelCreate { Rendezvous|Broadcast, T0 }`
//! - `queue!(N)` → `ChannelCreate { Buffered(N), T0 }`
//! - `fork!(action, args)` → `Fork { action_name, args }`

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};

use crate::typed::{ChannelKind, TypedExpr, TypedExprKind};

use super::action_call::ActionDispatch;
use super::expr::InferCtx;
use super::helpers::InferResult;

/// `channel!()` e `broadcast!()` — sem argumentos além de `()`.
///
/// Cria `ChannelCreate { kind, elem_ty: Var("T0") }` e retorna a tupla
/// apropriada:
/// - Rendezvous/Buffered: `(Sender::T0, Receiver::T0)`
/// - Broadcast: `(Sender::T0, ReceiverFactory::T0)`
pub(crate) fn infer_channel_builtin(
    kind: ChannelKind,
    args: &Spanned<Expr>,
    span: &Span,
) -> InferResult<TypedExpr> {
    // args deve ser Unit (tupla vazia `()`).
    if !matches!(args.node, Expr::Unit) {
        return Err(kata_diagnostics::MiddleError::TypeMismatch {
            expected: "`()` — channel!()/broadcast!() não recebem argumentos".into(),
            found: format!("{:?}", args.node),
            span: args.span.into(),
        });
    }

    let elem_ty = Ty::Var("T0".into());
    let ret_ty = match kind {
        ChannelKind::Rendezvous | ChannelKind::Buffered(_) => Ty::Tuple(vec![
            Ty::Sender(Box::new(elem_ty.clone())),
            Ty::Receiver(Box::new(elem_ty.clone())),
        ]),
        ChannelKind::Broadcast => Ty::Tuple(vec![
            Ty::Sender(Box::new(elem_ty.clone())),
            Ty::ReceiverFactory(Box::new(elem_ty.clone())),
        ]),
    };

    Ok(TypedExpr {
        span: *span,
        ty: ret_ty,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::ChannelCreate { kind, elem_ty },
    })
}

/// `queue!(N)` — N deve ser Int literal positivo.
pub(crate) fn infer_queue_builtin(
    args: &Spanned<Expr>,
    span: &Span,
) -> InferResult<ActionDispatch> {
    // args deve ser uma tupla/Grouping com 1 elemento Int.
    let capacity = match &args.node {
        Expr::Grouping { inner } => match &inner.node {
            Expr::IntLit { text } => text.parse::<i64>().unwrap_or(0),
            other => {
                return Err(kata_diagnostics::MiddleError::TypeMismatch {
                    expected: "Int literal (capacidade do queue)".into(),
                    found: format!("{other:?}"),
                    span: inner.span.into(),
                });
            }
        },
        Expr::IntLit { text } => text.parse::<i64>().unwrap_or(0),
        other => {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: "Int literal (capacidade do queue)".into(),
                found: format!("{other:?}"),
                span: args.span.into(),
            });
        }
    };

    if capacity <= 0 {
        return Err(kata_diagnostics::MiddleError::TypeMismatch {
            expected: "Int positivo (capacidade do queue > 0)".into(),
            found: capacity.to_string(),
            span: args.span.into(),
        });
    }

    let elem_ty = Ty::Var("T0".into());
    let ret_ty = Ty::Tuple(vec![
        Ty::Sender(Box::new(elem_ty.clone())),
        Ty::Receiver(Box::new(elem_ty.clone())),
    ]);

    Ok(ActionDispatch::Complete(TypedExpr {
        span: *span,
        ty: ret_ty,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::ChannelCreate {
            kind: ChannelKind::Buffered(capacity),
            elem_ty,
        },
    }))
}

/// `fork!(action_name, (arg1, arg2, ...))` — spawn de fiber.
///
/// Verifica que `action_name` é nome de Action declarada no DispatchTable
/// e que os args matcham os params da Action. Retorna `Unit`, effect `Spawn`.
pub(crate) fn infer_fork_builtin(
    args: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    // args é uma tupla de 2 elementos: (action_name, args_tuple).
    let elements = match &args.node {
        Expr::Tuple { elements } => elements,
        other => {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: "tupla (action_name, args) para fork!".into(),
                found: format!("{other:?}"),
                span: args.span.into(),
            });
        }
    };

    if elements.len() != 2 {
        return Err(kata_diagnostics::MiddleError::TypeMismatch {
            expected: "tupla de 2 elementos: (action_name, args)".into(),
            found: format!("{} elementos", elements.len()),
            span: args.span.into(),
        });
    }

    // Primeiro elemento: nome da Action (Ident) ou variável do tipo Action.
    // Inference: infer the expression to get a TypedExpr for action_expr.
    let action_expr_typed =
        super::expr::infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;

    // Determine action_name and whether this is a direct or indirect fork.
    // Direct: `fork!(worker, ...)` — worker is an Action in the DispatchTable.
    // Indirect: `fork!(f, ...)` — f is a variable holding Ty::Action.
    let (action_name, is_direct) = match &elements[0].node {
        Expr::Ident { name } => {
            // Check if it's a variable in env (indirect) or a DispatchTable action (direct).
            if env.lookup(name).is_some() {
                // Variable — indirect fork via action_expr.
                ("__indirect_fork".to_string(), false)
            } else {
                // DispatchTable action — direct fork.
                (name.clone(), true)
            }
        }
        _ => {
            // Non-Ident expression — always indirect.
            ("__indirect_fork".to_string(), false)
        }
    };

    if is_direct {
        // Verifica que action_name é uma Action declarada.
        if !ctx.table.has_function(&action_name) {
            return Err(kata_diagnostics::MiddleError::UnboundName {
                name: format!("Action `{action_name}` não declarada (fork!)"),
                span: elements[0].span.into(),
            });
        }

        // Verifica que é uma Action (is_action = true).
        let overloads = ctx.table.get_overloads(&action_name).ok_or_else(|| {
            kata_diagnostics::MiddleError::UnboundName {
                name: format!("Action `{action_name}` não tem overloads"),
                span: elements[0].span.into(),
            }
        })?;

        let any_action = overloads.iter().any(|o| o.is_action);
        if !any_action {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: format!("Action `{action_name}` (is_action=true)"),
                found: format!("`{action_name}` é função pura, não Action"),
                span: elements[0].span.into(),
            });
        }
    } else {
        // Indirect fork: verify action_expr_typed has Ty::Action.
        if !matches!(&action_expr_typed.ty, Ty::Action(_, _)) {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: "Action (fn_ptr) como primeiro arg de fork!".into(),
                found: format!("{:?}", action_expr_typed.ty),
                span: elements[0].span.into(),
            });
        }
    }

    // Segundo elemento: tupla de argumentos para a Action.
    // Infere o tipo para que o codegen saiba quantos args passar.
    let typed_args =
        super::expr::infer_expr(&elements[1].node, &elements[1].span, env, ctx, false)?;

    // Normaliza Grouping → Tuple (mesmo que action_call.rs principal).
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
                escape: typed_args.escape,
            }
        }
        _ => typed_args,
    };

    Ok(ActionDispatch::Complete(TypedExpr {
        span: *span,
        ty: Ty::Unit,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Fork {
            action_name,
            action_expr: Box::new(Spanned::new(action_expr_typed, elements[0].span)),
            args: Box::new(Spanned::new(typed_args, elements[1].span)),
        },
    }))
}
