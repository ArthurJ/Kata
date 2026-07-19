//! ActionCall — dispatch para Action builtin ou definida pelo usuário.
//!
//! Extraído de `expr.rs` — o braço `Expr::ActionCall` é self-contained:
//! chama `infer_expr` (para inferir args) e `infer_assert` (de sugar.rs),
//! mas não chama `infer_expr_hinted` recursivamente.
//!
//! Retorna `Result<ExprDispatch, MiddleError>` onde `ExprDispatch` é ou
//! um `TypedExpr` completo (assert — early return) ou a tríade
//! `(Ty, TypedExprKind, Effect)` consumida pelo match principal.

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};

use crate::typed::{ChannelKind, Effect, TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;
use super::sugar::infer_assert;

/// Resultado da inferência de ActionCall.
///
/// `Complete(TypedExpr)` = early return com TypedExpr pronto (ex: assert).
/// `Tuple(ty, kind, effect)` = tríade para o match principal montar o TypedExpr.
pub(crate) enum ActionDispatch {
    Complete(TypedExpr),
    Tuple(Ty, TypedExprKind, Effect),
}

/// Infere um `Expr::ActionCall { callee, args }`.
///
/// Builtins interceptados antes do DispatchTable:
/// - `assert!` → desugar para match
/// - `channel!()` → `ChannelCreate { Rendezvous, T0 }`
/// - `queue!(N)` → `ChannelCreate { Buffered(N), T0 }`
/// - `broadcast!()` → `ChannelCreate { Broadcast, T0 }`
/// - `fork!(action, args)` → `Fork { action_name, args }`
///
/// Caso especial: se `callee` não está no DispatchTable mas é uma variável
/// local do tipo `ReceiverFactory::T`, despacha como receiver factory call
/// (produz `Receiver::T`).
pub(crate) fn infer_action_call(
    callee: &str,
    args: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    // Assert! é desugared no typeck para
    // match cond { True: Unit, False: panic!(msg) }.
    if callee == "assert" {
        let typed = infer_assert(args, span, env, ctx)?;
        return Ok(ActionDispatch::Complete(typed));
    }

    // ── Builtins CSP ──
    if callee == "channel" {
        return Ok(ActionDispatch::Complete(infer_channel_builtin(
            ChannelKind::Rendezvous,
            args,
            span,
        )?));
    }
    if callee == "queue" {
        return infer_queue_builtin(args, span);
    }
    if callee == "broadcast" {
        return Ok(ActionDispatch::Complete(infer_channel_builtin(
            ChannelKind::Broadcast,
            args,
            span,
        )?));
    }
    if callee == "fork" {
        return infer_fork_builtin(args, span, env, ctx);
    }

    // ── Receiver factory call ──
    // `rxf!()` onde `rxf` é uma variável do tipo `ReceiverFactory::T`.
    // O callee não é um builtin nomeado — é o nome da variável.
    // Se não está no DispatchTable mas é ReceiverFactory no env, despacha.
    //
    // Constrói `TypedExprKind::ReceiverFactoryCall { factory, elem_ty }`
    // (não `ChannelCreate { Broadcast }` — aquela cria um broadcast novo;
    // esta pede um receiver a um factory existente). O codegen lowera
    // para `kata_rt_broadcast_receiver_create(arena, factory_handle)`.
    if !ctx.table.has_function(callee)
        && let Some(ty) = env.lookup(callee)
        && let Ty::ReceiverFactory(inner) = ty
    {
        // args deve ser Unit (`rxf!()` não recebe argumentos).
        if !matches!(args.node, Expr::Unit) {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: "`()` — rxf!() não recebe argumentos".into(),
                found: format!("{:?}", args.node),
                span: args.span.into(),
            });
        }
        // Expressão factory: Ident do callee, tipada como ReceiverFactory.
        let factory_typed = TypedExpr {
            span: *span,
            ty: ty.clone(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::Ident {
                name: callee.to_string(),
            },
        };
        let typed = TypedExpr {
            span: *span,
            ty: Ty::Receiver(Box::new((**inner).clone())),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            effect: Effect::ChannelOp,
            kind: TypedExprKind::ReceiverFactoryCall {
                factory: Box::new(Spanned::new(factory_typed, *span)),
                elem_ty: (**inner).clone(),
            },
        };
        return Ok(ActionDispatch::Complete(typed));
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
                escape: typed_args.escape,
                effect: typed_args.effect,
            }
        }
        _ => typed_args,
    };

    // Extrai tipos dos elementos da tupla para dispatch.
    let arg_tys: Vec<Ty> = match &typed_args.kind {
        TypedExprKind::Tuple { elements } => elements.iter().map(|e| e.node.ty.clone()).collect(),
        TypedExprKind::Unit => Vec::new(), // `!()` = tupla vazia
        _ => vec![typed_args.ty.clone()],  // args não-tupla (não deveria acontecer)
    };

    // Resolve no DispatchTable.
    let overload = ctx
        .table
        .resolve(callee, &arg_tys, ctx.interface_registry)
        .map_err(|e| super::helpers::dispatch_to_middle_error(e, *span))?;

    // Verifica que é uma Action (is_action = true).
    if !overload.is_action {
        return Err(kata_diagnostics::MiddleError::TypeMismatch {
            expected: format!("Action `{callee}` (is_action=true)"),
            found: format!("função pura `{callee}` — use sem `!`"),
            span: (*span).into(),
        });
    }

    Ok(ActionDispatch::Tuple(
        overload.ret,
        TypedExprKind::ActionCall {
            callee: callee.to_string(),
            args: Box::new(Spanned::new(typed_args, args.span)),
            caller_arena: 0, // placeholder — preenchido no codegen
            ffi_symbol: overload.ffi_symbol.clone().filter(|_s| overload.is_action),
        },
        Effect::Puro, // Não ativa Effect
    ))
}

/// `channel!()` e `broadcast!()` — sem argumentos além de `()`.
///
/// Cria `ChannelCreate { kind, elem_ty: Var("T0") }` e retorna a tupla
/// apropriada:
/// - Rendezvous/Buffered: `(Sender::T0, Receiver::T0)`
/// - Broadcast: `(Sender::T0, ReceiverFactory::T0)`
fn infer_channel_builtin(
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
        effect: Effect::ChannelOp,
        kind: TypedExprKind::ChannelCreate { kind, elem_ty },
    })
}

/// `queue!(N)` — N deve ser Int literal positivo.
fn infer_queue_builtin(args: &Spanned<Expr>, span: &Span) -> InferResult<ActionDispatch> {
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
        effect: Effect::ChannelOp,
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
fn infer_fork_builtin(
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

    // Primeiro elemento: nome da Action (Ident).
    let action_name = match &elements[0].node {
        Expr::Ident { name } => name.clone(),
        other => {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: "Ident (nome da Action) como primeiro arg de fork!".into(),
                found: format!("{other:?}"),
                span: elements[0].span.into(),
            });
        }
    };

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
                effect: typed_args.effect,
            }
        }
        _ => typed_args,
    };

    Ok(ActionDispatch::Complete(TypedExpr {
        span: *span,
        ty: Ty::Unit,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        effect: Effect::Spawn,
        kind: TypedExprKind::Fork {
            action_name,
            args: Box::new(Spanned::new(typed_args, elements[1].span)),
        },
    }))
}
