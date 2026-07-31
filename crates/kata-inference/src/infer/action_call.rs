//! ActionCall — dispatch para Action builtin ou definida pelo usuário.
//!
//! Extraído de `expr.rs` — o braço `Expr::ActionCall` é self-contained:
//! chama `infer_expr` (para inferir args) e `infer_assert` (de sugar.rs),
//! mas não chama `infer_expr_hinted` recursivamente.
//!
//! Retorna `Result<ExprDispatch, MiddleError>` onde `ExprDispatch` é ou
//! um `TypedExpr` completo (assert — early return) ou o par
//! `(Ty, TypedExprKind)` consumido pelo match principal.

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};

use crate::typed::{TypedExpr, TypedExprKind};

use super::csp_builtins::{
    infer_channel_builtin, infer_fork_builtin, infer_queue_builtin, infer_spawn_builtin,
};
use super::expr::{InferCtx, infer_expr};
use super::helpers::{InferResult, reorder_dict_args_to_tuple};
use super::log_builtins::{infer_log_builtin, infer_log_config_builtin, infer_log_recv_builtin};
use super::sugar::infer_assert;

/// Resultado da inferência de ActionCall.
///
/// `Complete(TypedExpr)` = early return com TypedExpr pronto (ex: assert).
/// `Tuple(ty, kind)` = par para o match principal montar o TypedExpr.
pub(crate) enum ActionDispatch {
    Complete(TypedExpr),
    Tuple(Ty, TypedExprKind),
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
            crate::typed::ChannelKind::Rendezvous,
            args,
            span,
        )?));
    }
    if callee == "queue" {
        return infer_queue_builtin(args, span);
    }
    if callee == "broadcast" {
        return Ok(ActionDispatch::Complete(infer_channel_builtin(
            crate::typed::ChannelKind::Broadcast,
            args,
            span,
        )?));
    }
    if callee == "fork" {
        return infer_fork_builtin(args, span, env, ctx);
    }
    if callee == "spawn" {
        return infer_spawn_builtin(args, span, env, ctx);
    }

    // ── Builtins Log ──
    if callee == "log" {
        return infer_log_builtin(args, span, env, ctx);
    }
    if callee == "log_recv" {
        return infer_log_recv_builtin(args, span, env, ctx);
    }
    if callee == "log_config" {
        return infer_log_config_builtin(args, span, env, ctx);
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
            kind: TypedExprKind::Ident {
                name: callee.to_string(),
            },
        };
        let typed = TypedExpr {
            span: *span,
            ty: Ty::Receiver(Box::new((**inner).clone())),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::ReceiverFactoryCall {
                factory: Box::new(Spanned::new(factory_typed, *span)),
                elem_ty: (**inner).clone(),
            },
        };
        return Ok(ActionDispatch::Complete(typed));
    }

    // ── Indirect Action invocation ──
    // `f!(args)` onde `f` é variável local com `ty: Ty::Action(params, ret)`.
    // O callee não está no DispatchTable — é o nome de uma variável.
    if !ctx.table.has_function(callee)
        && let Some(ty) = env.lookup(callee).cloned()
        && let Ty::Action(param_types, ret_ty) = ty
    {
        // Lowera a tupla de argumentos.
        let typed_args = infer_expr(&args.node, &args.span, env, ctx, false)?;

        // Normaliza Grouping → Tuple de 1 elemento, e args não-tupla → Tuple
        // de 1 elemento. O codegen precisa de Tuple (ponteiro para array na
        // arena) para passar args_ptr corretamente ao call_indirect.
        // Sem isso, `job!(payload)` passa o valor bruto (ex: 42) como args_ptr,
        // e o callee tenta load de endereço inválido → SIGSEGV.
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
            TypedExprKind::Tuple { .. } | TypedExprKind::Unit => typed_args,
            _ => {
                // Args não-tupla (ex: Ident solo, Int literal) → Tuple de 1.
                TypedExpr {
                    ty: Ty::Tuple(vec![typed_args.ty.clone()]),
                    kind: TypedExprKind::Tuple {
                        elements: vec![Spanned::new(typed_args.clone(), args.span)],
                    },
                    span: typed_args.span,
                    tail_pos: typed_args.tail_pos,
                    escape: typed_args.escape,
                }
            }
        };

        // Valida que args matcham param_types.
        let arg_tys: Vec<Ty> = match &typed_args.kind {
            TypedExprKind::Tuple { elements } => {
                elements.iter().map(|e| e.node.ty.clone()).collect()
            }
            TypedExprKind::Unit => Vec::new(),
            _ => vec![typed_args.ty.clone()],
        };
        if arg_tys.len() != param_types.len() {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: format!(
                    "{} args para Action com {} params",
                    param_types.len(),
                    param_types.len()
                ),
                found: format!("{} args", arg_tys.len()),
                span: (*span).into(),
            });
        }
        for (actual, expected) in arg_tys.iter().zip(param_types.iter()) {
            if actual != expected {
                return Err(kata_diagnostics::MiddleError::TypeMismatch {
                    expected: format!("{expected}"),
                    found: format!("{actual}"),
                    span: (*span).into(),
                });
            }
        }

        // Constrói a expressão do callee (Ident com ty: Ty::Action).
        let callee_expr = TypedExpr {
            span: *span,
            ty: Ty::Action(param_types.clone(), ret_ty.clone()),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::Ident {
                name: callee.to_string(),
            },
        };

        return Ok(ActionDispatch::Complete(TypedExpr {
            span: *span,
            ty: (*ret_ty).clone(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::ActionCall {
                callee: callee.to_string(),
                args: Box::new(Spanned::new(typed_args, args.span)),
                caller_arena: 0,
                ffi_symbol: None,
                indirect_callee: Some(Box::new(Spanned::new(callee_expr, *span))),
            },
        }));
    }

    // Lowera a tupla de argumentos.
    let typed_args = infer_expr(&args.node, &args.span, env, ctx, false)?;

    // Se args é DictLit, mapeia chaves → nomes de params e reordena para Tuple.
    // `g!{"b": 2 "a": 1}` → Tuple [1, 2] na ordem posicional dos params de g.
    let typed_args = match &typed_args.kind {
        TypedExprKind::DictLit { entries, .. } => {
            reorder_dict_args_to_tuple(callee, entries, &typed_args, ctx, *span)?
        }
        _ => typed_args,
    };

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

    // Aplica defaults do EnumRegistry no tipo de retorno.
    // O DispatchTable guarda o ret da assinatura (ex: Result::A com arity 1);
    // expand_defaults preenche E|Text para que a chave do codegen
    // (name, params, ret) bata com a action registrada.
    let expanded_ret = ctx.enum_registry.expand_defaults(&overload.ret);

    Ok(ActionDispatch::Tuple(
        expanded_ret,
        TypedExprKind::ActionCall {
            callee: callee.to_string(),
            args: Box::new(Spanned::new(typed_args, args.span)),
            caller_arena: 0, // placeholder — preenchido no codegen
            ffi_symbol: overload.ffi_symbol.clone().filter(|_s| overload.is_action),
            indirect_callee: None,
        },
    ))
}
