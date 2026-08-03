//! Typeck de expressões CSP.
//!
//! `ChannelSend` (`!>`), `ChannelRecv` (`<!`), e `Select` são inferidos aqui.
//! `channel!()`, `queue!()`, `broadcast!()`, `rxf!()`, `fork!()` são
//! interceptados em `infer_apply` (não despacham para DispatchTable).

use kata_ast::{Expr, SelectArm, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind, TypedSelectArm};

use super::expr::InferCtx;
use super::expr::infer_expr_hinted;
use super::helpers::InferResult;

/// `tx !> valor` — envio por canal.
///
/// `channel` deve ter tipo `Sender::T`. `value` deve ter tipo `T`.
/// Produz `Unit` (envio é side-effect). Effect = `ChannelOp`.
pub(crate) fn infer_channel_send(
    channel: &Spanned<Expr>,
    value: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let typed_channel = infer_expr_hinted(&channel.node, &channel.span, env, ctx, false, None)?;

    // Verifica que channel é Sender::T.
    let elem_ty = match &typed_channel.ty {
        Ty::Sender(inner) => (**inner).clone(),
        other => {
            return Err(MiddleError::TypeMismatch {
                expected: "Sender::T (canal sender)".into(),
                found: format!("{other:?}"),
                span: (*span).into(),
            });
        }
    };

    // value deve ser do tipo T (o tipo do canal).
    let typed_value = infer_expr_hinted(&value.node, &value.span, env, ctx, false, Some(&elem_ty))?;

    // Proibe Ty::Action em canal — Actions são comportamento, não informação
    // que possa viajar por um canal. (PRD §3.7)
    if let Ty::Action(..) = &typed_value.ty {
        return Err(MiddleError::TypeMismatch {
            expected: "valor serializável (não-Action)".into(),
            found: format!(
                "Action não é permitida em canal — Actions são comportamento, não informação. \
                 Tipo do valor: `{}`",
                typed_value.ty
            ),
            span: value.span.into(),
        });
    }

    if !type_compatible(&typed_value.ty, &elem_ty) {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{elem_ty:?}"),
            found: format!("{}", typed_value.ty),
            span: value.span.into(),
        });
    }

    let escape = escape_for_channel_send(&typed_value.ty, tail_pos, ctx);

    // Override o escape do typed_value: se o valor é composto, precisa ser
    // alocado na caller_arena (arena do pai) para sobreviver ao fiber
    // que o envia. O inference do typed_value usou tail_pos=false → Local,
    // mas o channel send exige que o valor sobreviva além do sender.
    let typed_value = if escape != typed_value.escape {
        TypedExpr {
            escape,
            ..typed_value
        }
    } else {
        typed_value
    };

    Ok(TypedExpr {
        span: *span,
        ty: Ty::Unit,
        tail_pos,
        escape,
        kind: TypedExprKind::ChannelSend {
            channel: Box::new(Spanned::new(typed_channel, channel.span)),
            value: Box::new(Spanned::new(typed_value, value.span)),
        },
    })
}

/// `rx <! nome` — recebimento de canal.
///
/// `channel` deve ter tipo `Receiver::T`. Infere `T` e cria binding
/// `bind_name: T` no `TypeEnv`. Produz `T` (o valor recebido).
pub(crate) fn infer_channel_recv(
    channel: &Spanned<Expr>,
    bind_name: &str,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let typed_channel = infer_expr_hinted(&channel.node, &channel.span, env, ctx, false, None)?;

    // Verifica que channel é Receiver::T e extrai T.
    let recv_ty = match &typed_channel.ty {
        Ty::Receiver(inner) => (**inner).clone(),
        other => {
            return Err(MiddleError::TypeMismatch {
                expected: "Receiver::T (canal receiver)".into(),
                found: format!("{other:?}"),
                span: (*span).into(),
            });
        }
    };

    // Cria binding no TypeEnv: nome := recv_ty.
    env.define(bind_name, recv_ty.clone(), "__local__");

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Caller
    };

    Ok(TypedExpr {
        span: *span,
        ty: recv_ty.clone(),
        tail_pos,
        escape,
        kind: TypedExprKind::ChannelRecv {
            channel: Box::new(Spanned::new(typed_channel, channel.span)),
            recv_ty,
            bind_name: bind_name.to_string(),
        },
    })
}

/// `select` com braços de canal e timeout opcional.
///
/// Todos os braços devem ter receivers do mesmo tipo `T`. O tipo do
/// `select` é `T` (o valor recebido pelo braço que disparar).
pub(crate) fn infer_select(
    arms: &[SelectArm],
    timeout_ms: &Option<Box<Spanned<Expr>>>,
    timeout_body: &Option<Box<Spanned<Expr>>>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let mut typed_arms: Vec<TypedSelectArm> = Vec::new();
    let mut unified_ty: Option<Ty> = None;

    for arm in arms {
        let typed_channel =
            infer_expr_hinted(&arm.channel.node, &arm.channel.span, env, ctx, false, None)?;

        // Verifica que channel é Receiver::T.
        let recv_ty = match &typed_channel.ty {
            Ty::Receiver(inner) => (**inner).clone(),
            other => {
                return Err(MiddleError::TypeMismatch {
                    expected: "Receiver::T (canal receiver)".into(),
                    found: format!("{other:?}"),
                    span: arm.channel.span.into(),
                });
            }
        };

        // Unifica tipo entre braços.
        if let Some(ref existing) = unified_ty {
            if !type_compatible(&recv_ty, existing) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{existing:?} (tipo do primeiro braço do select)"),
                    found: format!("{recv_ty:?}"),
                    span: arm.channel.span.into(),
                });
            }
        } else {
            unified_ty = Some(recv_ty.clone());
        }

        // Cria binding do braço num escopo filho.
        let mut arm_env = env.push_scope();
        arm_env.define(&arm.bind_name, recv_ty.clone(), "__local__");

        let typed_body = infer_expr_hinted(
            &arm.body.node,
            &arm.body.span,
            &mut arm_env,
            ctx,
            tail_pos,
            None,
        )?;

        typed_arms.push(TypedSelectArm {
            channel: Spanned::new(typed_channel, arm.channel.span),
            recv_ty: recv_ty.clone(),
            bind_name: arm.bind_name.clone(),
            body: Spanned::new(typed_body, arm.body.span),
        });
    }

    let select_ty = unified_ty.unwrap_or(Ty::Unit);

    // Typeck do timeout.
    let mut typed_timeout_ms = None;
    let mut typed_timeout_body = None;

    if let Some(tm) = timeout_ms {
        let tm_typed = infer_expr_hinted(&tm.node, &tm.span, env, ctx, false, None)?;
        // timeout_ms deve ser Int.
        if !type_compatible(&tm_typed.ty, &Ty::int()) {
            return Err(MiddleError::TypeMismatch {
                expected: "Int (timeout em milissegundos)".into(),
                found: format!("{}", tm_typed.ty),
                span: tm.span.into(),
            });
        }
        typed_timeout_ms = Some(Box::new(Spanned::new(tm_typed, tm.span)));
    }

    if let (Some(tb), Some(_)) = (timeout_body, &typed_timeout_ms) {
        let tb_typed = infer_expr_hinted(&tb.node, &tb.span, env, ctx, tail_pos, None)?;
        // timeout_body deve produzir o mesmo tipo que os braços.
        if !type_compatible(&tb_typed.ty, &select_ty) {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{select_ty:?} (tipo do select)"),
                found: format!("{}", tb_typed.ty),
                span: tb.span.into(),
            });
        }
        typed_timeout_body = Some(Box::new(Spanned::new(tb_typed, tb.span)));
    }

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Caller
    };

    Ok(TypedExpr {
        span: *span,
        ty: select_ty,
        tail_pos,
        escape,
        kind: TypedExprKind::Select {
            arms: typed_arms,
            timeout_ms: typed_timeout_ms,
            timeout_body: typed_timeout_body,
        },
    })
}

/// Verifica compatibilidade de tipos — estrutural para tipos concretos,
/// aceita Var como coringa (para type params não-resolvidos).
fn type_compatible(actual: &Ty, expected: &Ty) -> bool {
    if actual == expected {
        return true;
    }
    // Var unifica com qualquer tipo (mesma semântica de fits_return).
    matches!(actual, Ty::Var(_)) || matches!(expected, Ty::Var(_))
}

/// Escape target para `!>` — valor escapa para outro fiber.
///
/// Tipos compostos (Tuple, Struct, List, Array, Dict, Set, Text, etc.)
/// são alocados na arena e precisam sobreviver ao sender → Heap.
/// Tipos primitivos (Int/SMI, Float, Boolean, Unit) são inline (i64)
/// e não precisam de ARC → Local (sem overhead).
fn escape_for_channel_send(ty: &Ty, _tail_pos: bool, _ctx: &InferCtx) -> EscapeTarget {
    match ty {
        // Primitivos inline — sem alocação.
        Ty::Prim(_) | Ty::Unit => EscapeTarget::Local,
        // Action não pode viajar por canal (validado em infer_channel_send).
        Ty::Action(..) => EscapeTarget::Local,
        // Var/InferVar — conservador: Local (não sabemos o tipo concreto).
        Ty::Var(_) | Ty::InferVar(_) => EscapeTarget::Local,
        // Sender/Receiver são handles (i64), não ponteiros.
        Ty::Sender(_) | Ty::Receiver(_) => EscapeTarget::Local,
        // Function é fn_ptr, não ponteiro.
        Ty::Function(..) => EscapeTarget::Local,
        // Compostos — alocados na caller_arena, que sobrevive ao fiber
        // que os envia. O scheduler é structured concurrency: o pai só
        // morre depois de todas as filhas, então a caller_arena (arena
        // do pai) cobre o lifetime de todos os interessados no valor.
        _ => EscapeTarget::Caller,
    }
}
