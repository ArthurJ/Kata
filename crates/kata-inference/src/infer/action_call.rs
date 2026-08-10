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
use super::timer_builtins::infer_now_builtin;

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

    // ── Builtins Timer ──
    if callee == "now" {
        return infer_now_builtin(args, span);
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

    // ── Indirect Action invocation via OverloadSet ──
    // `f!(args)` onde `f` é variável local com `ty: Ty::OverloadSet`.
    // O callee não está no DispatchTable — é o nome de uma variável.
    // Faz dispatch por args: usa match_score para selecionar o overload
    // compatível entre os overloads do OverloadSet.
    if !ctx.table.has_function(callee)
        && let Some(ty) = env.lookup(callee).cloned()
        && let Ty::OverloadSet {
            name: action_name,
            overloads,
        } = &ty
    {
        // Lowera a tupla de argumentos.
        let typed_args = infer_expr(&args.node, &args.span, env, ctx, false)?;

        // Normaliza Grouping → Tuple de 1 elemento, e args não-tupla → Tuple.
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
            _ => TypedExpr {
                ty: Ty::Tuple(vec![typed_args.ty.clone()]),
                kind: TypedExprKind::Tuple {
                    elements: vec![Spanned::new(typed_args.clone(), args.span)],
                },
                span: typed_args.span,
                tail_pos: typed_args.tail_pos,
                escape: typed_args.escape,
            },
        };

        // Extrai tipos dos args.
        let arg_tys: Vec<Ty> = match &typed_args.kind {
            TypedExprKind::Tuple { elements } => {
                elements.iter().map(|e| e.node.ty.clone()).collect()
            }
            TypedExprKind::Unit => Vec::new(),
            _ => vec![typed_args.ty.clone()],
        };

        // Dispatch por args: filtra overloads compatíveis por match_score.
        use kata_core::dispatch::match_score;
        let compatibles: Vec<&(Vec<Ty>, Ty)> = overloads
            .iter()
            .filter(|(params, _)| {
                if params.len() != arg_tys.len() {
                    return false;
                }
                let score = match_score(&arg_tys, params, ctx.interface_registry);
                score.is_compatible(arg_tys.len())
            })
            .collect();

        if compatibles.is_empty() {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: format!("args compatíveis com algum overload de `{action_name}`"),
                found: format!(
                    "nenhum overload casa com args de tipos [{}]",
                    arg_tys
                        .iter()
                        .map(|t| t.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                span: (*span).into(),
            });
        }
        if compatibles.len() > 1 {
            return Err(kata_diagnostics::MiddleError::AmbiguousDispatch {
                name: action_name.clone(),
                span: (*span).into(),
            });
        }

        // overload único compatível — resolve para Ty::Action concreto.
        let (_, ret) = compatibles[0];

        // Constrói ActionCall com callee = action_name (do OverloadSet, não da variável).
        // O codegen faz lookup por action_name + args no DispatchTable/kata_refs.
        return Ok(ActionDispatch::Complete(TypedExpr {
            span: *span,
            ty: ret.clone(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::ActionCall {
                callee: action_name.clone(),
                args: Box::new(Spanned::new(typed_args, args.span)),
                caller_arena: 0,
                ffi_symbol: None,
                indirect_callee: None,
            },
        }));
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
        // Valida args contra param_types via match_score (interface dispatch).
        // Antes usava == estrito, que falhava quando param é Interface("SHOW")
        // e arg é Text (Text implementa SHOW). match_score verifica compatibilidade
        // via InterfaceRegistry, permitindo dispatch por interface no caminho indirect.
        use kata_core::dispatch::match_score;
        let score = match_score(&arg_tys, &param_types, ctx.interface_registry);
        if !score.is_compatible(arg_tys.len()) {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: param_types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                found: arg_tys
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                span: (*span).into(),
            });
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
                // callee = nome da action original (via fn_alias), não o nome
                // da variável. O monomorphizador precisa do nome da action
                // para encontrar overloads no DispatchTable e instanciar a
                // versão genérica. Se não há alias (variável não vem de
                // `let f := action`), fallback para o próprio callee.
                callee: env.fn_alias_of(callee).unwrap_or(callee).to_string(),
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
    let mut typed_args = match &typed_args.kind {
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
    let outcome = ctx
        .table
        .resolve_with_swap(callee, &arg_tys, ctx.interface_registry)
        .map_err(|e| super::helpers::dispatch_to_middle_error(e, *span))?;
    let overload = outcome.overload;

    // Verifica que é uma Action (is_action = true).
    if !overload.is_action {
        return Err(kata_diagnostics::MiddleError::TypeMismatch {
            expected: format!("Action `{callee}` (is_action=true)"),
            found: format!("função pura `{callee}` — use sem `!`"),
            span: (*span).into(),
        });
    }

    // Reordenar elementos da tupla de args se o dispatch resolveu via
    // commutative swap. O codegen desempacota a tupla na ordem posicional,
    // então os elementos precisam estar na ordem esperada pela overload.
    if outcome.swapped {
        typed_args = match typed_args.kind.clone() {
            TypedExprKind::Tuple { elements } if elements.len() == 2 => {
                let mut swapped_elements = elements;
                swapped_elements.swap(0, 1);
                TypedExpr {
                    ty: typed_args.ty.clone(),
                    kind: TypedExprKind::Tuple {
                        elements: swapped_elements,
                    },
                    span: typed_args.span,
                    tail_pos: typed_args.tail_pos,
                    escape: typed_args.escape,
                }
            }
            _ => typed_args, // não-tupla ou ≠2 elementos — não reordena
        };
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
