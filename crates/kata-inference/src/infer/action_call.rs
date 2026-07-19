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
use kata_diagnostics::MiddleError;

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

    // ── Builtins Log (Fio 14) ──
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

// ── Log builtins (Fio 14) ──────────────────────────────────

/// `log!(level, msg, topic?, policy?)` — desugara para `kata_rt_log_publish`.
///
/// Args posicionais:
/// - 0: LogLevel (VariantQual ex: `LogLevel::Info` → tag i64)
/// - 1: Text (mensagem dinâmica)
/// - 2: Text (tópico, opcional → 0 = config herdada)
/// - 3: Text (policy, opcional → 0 = config herdada)
fn infer_log_builtin(
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
            effect: Effect::Puro,
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
            effect: Effect::Puro,
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
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_publish".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: Ty::int(),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Ancestor(0),
        effect: Effect::Puro,
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
fn infer_log_recv_builtin(
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
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_recv".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: Ty::text(),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Ancestor(0),
        effect: Effect::Puro,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, args.span)),
            args: vec![Spanned::new(topic_typed, elements[0].span)],
            ffi_symbol: Some("kata_rt_log_recv".into()),
        },
    };

    Ok(ActionDispatch::Complete(typed))
}

/// `log_config!(topic, policy, level)` — desugara para `kata_rt_log_config`.
fn infer_log_config_builtin(
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
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_config".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: Ty::Unit,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Ancestor(0),
        effect: Effect::Puro,
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
            effect: Effect::Puro,
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
            effect: Effect::Puro,
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
