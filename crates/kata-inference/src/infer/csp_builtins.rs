//! Builtins CSP (Communicating Sequential Processes) — channel, queue, fork.
//!
//! Extraído de `action_call.rs` — três funções self-contained que lidam com
//! os builtins de concorrência:
//! - `channel!()` / `broadcast!()` → `ChannelCreate { Rendezvous|Broadcast, T0 }`
//! - `queue!(N)` → `ChannelCreate { Buffered(N), T0 }`
//! - `fork!(action, args)` → `Fork { action_name, args }`

use std::cell::Cell;
use std::collections::HashMap;

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};

use crate::typed::{ChannelKind, TypedExpr, TypedExprKind};

use super::action_call::ActionDispatch;
use super::expr::InferCtx;
use super::helpers::InferResult;

// Contador global de type vars para channel!()/queue!()/broadcast!().
// Cada chamada cria um Var com nome único (T0, T1, T2, ...) para que
// a unificação de tipos no spawn!/fork! não colida entre canais diferentes.
thread_local! {
    static CHANNEL_TYPE_VAR_COUNTER: Cell<u32> = const { Cell::new(0) };
}

/// Gera um nome único para o type var de um canal.
fn next_channel_type_var() -> String {
    CHANNEL_TYPE_VAR_COUNTER.with(|c| {
        let n = c.get();
        c.set(n + 1);
        format!("T{n}")
    })
}

/// Reseta o contador de type vars de canal. Chamado no início de `infer_module`.
pub(crate) fn reset_channel_type_var_counter() {
    CHANNEL_TYPE_VAR_COUNTER.with(|c| c.set(0));
}

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

    let elem_ty = Ty::Var(next_channel_type_var());
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
        kind: TypedExprKind::ChannelCreate {
            kind,
            elem_ty,
            cross_process: false,
        },
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

    let elem_ty = Ty::Var(next_channel_type_var());
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
            cross_process: false,
        },
    }))
}

/// Extrai substituições de `Ty::Var` no `arg_ty` a partir do `param_ty`
/// (concreto). Direção inversa do `unify` em `generics.rs`: aqui o `param`
/// tem o tipo concreto e o `arg` contém `Var` a ser resolvida.
///
/// Ex: `param = Sender(List(Int))`, `arg = Sender(Var("T0"))` → subs["T0"] = List(Int).
fn extract_var_subs(param: &Ty, arg: &Ty, subs: &mut super::generics::Substitutions) {
    match (param, arg) {
        // Var no arg → ligar ao param concreto.
        (_, Ty::Var(name)) => {
            subs.entry(name.clone()).or_insert_with(|| param.clone());
        }
        // Sender/Receiver/ReceiverFactory — recursão no tipo interno.
        (Ty::Sender(p), Ty::Sender(a)) => extract_var_subs(p, a, subs),
        (Ty::Receiver(p), Ty::Receiver(a)) => extract_var_subs(p, a, subs),
        (Ty::ReceiverFactory(p), Ty::ReceiverFactory(a)) => extract_var_subs(p, a, subs),
        // List/Array/Range — recursão no elem.
        (Ty::List(p), Ty::List(a)) => extract_var_subs(p, a, subs),
        (Ty::Array(p), Ty::Array(a)) => extract_var_subs(p, a, subs),
        (Ty::Range(p), Ty::Range(a)) => extract_var_subs(p, a, subs),
        // Dict — recursão em K e V.
        (Ty::Dict(pk, pv), Ty::Dict(ak, av)) => {
            extract_var_subs(pk, ak, subs);
            extract_var_subs(pv, av, subs);
        }
        // Set — recursão no elem.
        (Ty::Set(p), Ty::Set(a)) => extract_var_subs(p, a, subs),
        // Tuple — recursão posicional.
        (Ty::Tuple(ps), Ty::Tuple(as_)) if ps.len() == as_.len() => {
            for (p, a) in ps.iter().zip(as_) {
                extract_var_subs(p, a, subs);
            }
        }
        // Generic — recursão nos args de tipo.
        (Ty::Generic(pn, ps), Ty::Generic(an, as_)) if pn == an && ps.len() == as_.len() => {
            for (p, a) in ps.iter().zip(as_) {
                extract_var_subs(p, a, subs);
            }
        }
        _ => {}
    }
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
    //
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

    // Inference do primeiro elemento só é necessária no caminho indirect
    // (para verificar que a variável tem Ty::Action). No caminho direct,
    // o fork faz seu próprio dispatch pelos args — inferir o Ident como
    // first-class ref falharia com AmbiguousDispatch para Actions overloadadas.
    let action_expr_typed: Option<TypedExpr> = if !is_direct {
        Some(super::expr::infer_expr(
            &elements[0].node,
            &elements[0].span,
            env,
            ctx,
            false,
        )?)
    } else {
        None
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
        let action_expr = action_expr_typed
            .as_ref()
            .expect("indirect fork tem action_expr_typed");
        if !matches!(&action_expr.ty, Ty::Action(_, _)) {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: "Action (fn_ptr) como primeiro arg de fork!".into(),
                found: format!("{:?}", action_expr.ty),
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

    // Unifica tipos dos args com tipos dos params da action.
    // Quando o arg contém Ty::Var (ex: Sender::T0 do channel!()) e o
    // param é concreto (ex: Sender::List::Int), extrai a substituição
    // T0 → List::Int e propaga para todos os bindings do env.
    // Isto resolve T0 para que rx <! lst produza lst: List::Int (não Var).
    if is_direct && let Some(overloads) = ctx.table.get_overloads(&action_name) {
        // Extrai tipos dos args.
        let arg_tys: Vec<Ty> = match &typed_args.kind {
            TypedExprKind::Tuple { elements } => {
                elements.iter().map(|e| e.node.ty.clone()).collect()
            }
            TypedExprKind::Unit => Vec::new(),
            _ => vec![typed_args.ty.clone()],
        };
        // Procura o overload com aridade correspondente.
        for oi in overloads
            .iter()
            .filter(|o| o.is_action && o.params.len() == arg_tys.len())
        {
            let mut subs: super::generics::Substitutions = HashMap::new();
            for (param, arg) in oi.params.iter().zip(&arg_tys) {
                extract_var_subs(param, arg, &mut subs);
            }
            if !subs.is_empty() {
                // Propaga substituições para todos os bindings do env.
                env.apply_substitutions(&subs);
                break;
            }
        }
    }

    // Para o Fork TAST node: no caso direct, o codegen não usa action_expr
    // (faz lookup por action_name em kata_ids). No caso indirect, lowera
    // action_expr para obter fn_ptr. Criamos um placeholder para direct.
    let placeholder_expr = TypedExpr {
        span: elements[0].span,
        ty: Ty::Unit,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Unit,
    };
    let action_expr_for_tast = action_expr_typed.unwrap_or(placeholder_expr);

    Ok(ActionDispatch::Complete(TypedExpr {
        span: *span,
        ty: Ty::Unit,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Fork {
            action_name,
            action_expr: Box::new(Spanned::new(action_expr_for_tast, elements[0].span)),
            args: Box::new(Spanned::new(typed_args, elements[1].span)),
        },
    }))
}

/// `spawn!(action_name, (arg1, arg2, ...))` — spawn de processo OS.
///
/// Diferença de fork!: `spawn!` cria um **processo OS separado** via
/// `fork()`. O child herda a arena via copy-on-write, executa a Action,
/// e termina com `_exit(0)`. Fire-and-forget — retorna `Unit`, não há
/// pipe de resultado. A comunicação entre parent e child é exclusivamente
/// por canais IPC (passados como args da Action).
///
/// O codegen chama `kata_rt_spawn_process(fn_ptr, args_ptr, arena_handle)`.
/// O child executa a Action diretamente (sem scheduler, sem fibers). Canais
/// IPC passados como args usam `kata_rt_ipc_channel_create` (pipe Unix) em
/// vez de `kata_rt_channel_create` (Mutex/Condvar) — marcado pelo pass
/// `cross_process.rs` na inferência.
///
/// Suporta forma posicional `spawn!(action, args)` e forma dict
/// `spawn!{callee: action, raw: args}`.
pub(crate) fn infer_spawn_builtin(
    args: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    // Extrai (action_expr, typed_args) de forma posicional ou dict.
    let (action_name, action_expr_typed, typed_args, args_span) = match &args.node {
        // Forma posicional: spawn!(action, (arg1, arg2, ...))
        Expr::Tuple { elements } => {
            if elements.len() != 2 {
                return Err(kata_diagnostics::MiddleError::TypeMismatch {
                    expected: "tupla de 2 elementos: (action_name, args)".into(),
                    found: format!("{} elementos", elements.len()),
                    span: args.span.into(),
                });
            }
            let action_expr_typed =
                super::expr::infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
            let (action_name, is_direct) = match &elements[0].node {
                Expr::Ident { name } => {
                    if env.lookup(name).is_some() {
                        ("__indirect_spawn".to_string(), false)
                    } else {
                        (name.clone(), true)
                    }
                }
                _ => ("__indirect_spawn".to_string(), false),
            };

            if is_direct && !ctx.table.has_function(&action_name) {
                return Err(kata_diagnostics::MiddleError::UnboundName {
                    name: format!("Action `{action_name}` não declarada (spawn!)"),
                    span: elements[0].span.into(),
                });
            }

            let typed_args =
                super::expr::infer_expr(&elements[1].node, &elements[1].span, env, ctx, false)?;
            let typed_args = normalize_grouping(typed_args);
            (action_name, action_expr_typed, typed_args, elements[1].span)
        }
        // Forma dict: spawn!{callee: action, raw: args}
        Expr::DictLit { entries } => {
            let mut callee_expr = None;
            let mut raw_expr = None;
            for (key, val) in entries {
                let key_name = match &key.node {
                    Expr::Ident { name } => name.clone(),
                    Expr::TextLit { text } => text.clone(),
                    _ => continue,
                };
                match key_name.as_str() {
                    "callee" => callee_expr = Some(val),
                    "raw" => raw_expr = Some(val),
                    "serialized" => {
                        // serialized: payload pré-serializado.
                        // Por ora, trata igual a raw (codegem decide).
                        raw_expr = Some(val);
                    }
                    _ => {}
                }
            }
            let callee =
                callee_expr.ok_or_else(|| kata_diagnostics::MiddleError::TypeMismatch {
                    expected: "chave `callee` em spawn!{...}".into(),
                    found: "dict sem chave `callee`".into(),
                    span: (*span).into(),
                })?;
            let raw = raw_expr.ok_or_else(|| kata_diagnostics::MiddleError::TypeMismatch {
                expected: "chave `raw` ou `serialized` em spawn!{...}".into(),
                found: "dict sem chave `raw`/`serialized`".into(),
                span: (*span).into(),
            })?;

            let action_expr_typed =
                super::expr::infer_expr(&callee.node, &callee.span, env, ctx, false)?;
            let (action_name, is_direct) = match &callee.node {
                Expr::Ident { name } => {
                    if env.lookup(name).is_some() {
                        ("__indirect_spawn".to_string(), false)
                    } else {
                        (name.clone(), true)
                    }
                }
                _ => ("__indirect_spawn".to_string(), false),
            };

            if is_direct && !ctx.table.has_function(&action_name) {
                return Err(kata_diagnostics::MiddleError::UnboundName {
                    name: format!("Action `{action_name}` não declarada (spawn!)"),
                    span: callee.span.into(),
                });
            }

            let typed_args = super::expr::infer_expr(&raw.node, &raw.span, env, ctx, false)?;
            let typed_args = normalize_grouping(typed_args);
            (action_name, action_expr_typed, typed_args, raw.span)
        }
        other => {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: "tupla (action, args) ou dict {callee: ..., raw: ...} para spawn!".into(),
                found: format!("{other:?}"),
                span: args.span.into(),
            });
        }
    };

    // Verifica que é uma Action (is_action = true) se direto.
    if !action_name.starts_with("__indirect") {
        let overloads = ctx.table.get_overloads(&action_name).ok_or_else(|| {
            kata_diagnostics::MiddleError::UnboundName {
                name: format!("Action `{action_name}` não tem overloads"),
                span: (*span).into(),
            }
        })?;
        let any_action = overloads.iter().any(|o| o.is_action);
        if !any_action {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: format!("Action `{action_name}` (is_action=true)"),
                found: format!("`{action_name}` é função pura, não Action"),
                span: (*span).into(),
            });
        }
    } else {
        // Indirect spawn: verify action_expr_typed has Ty::Action.
        if !matches!(&action_expr_typed.ty, Ty::Action(_, _)) {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: "Action (fn_ptr) como callee de spawn!".into(),
                found: format!("{:?}", action_expr_typed.ty),
                span: (*span).into(),
            });
        }
    }

    // Unifica tipos dos args com tipos dos params da action.
    // Mesmo mecanismo do fork!: quando o arg contém Ty::Var (ex:
    // Sender::T0 do channel!()) e o param é concreto (ex:
    // Sender::(Int, Int)), extrai a substituição T0 → (Int, Int) e
    // propaga para todos os bindings do env. Isto resolve T0 para que
    // o codegen veja o tipo concreto no ChannelCreate.
    if !action_name.starts_with("__indirect")
        && let Some(overloads) = ctx.table.get_overloads(&action_name)
    {
        let arg_tys: Vec<Ty> = match &typed_args.kind {
            TypedExprKind::Tuple { elements } => {
                elements.iter().map(|e| e.node.ty.clone()).collect()
            }
            TypedExprKind::Unit => Vec::new(),
            _ => vec![typed_args.ty.clone()],
        };
        for oi in overloads
            .iter()
            .filter(|o| o.is_action && o.params.len() == arg_tys.len())
        {
            let mut subs: super::generics::Substitutions = HashMap::new();
            for (param, arg) in oi.params.iter().zip(&arg_tys) {
                extract_var_subs(param, arg, &mut subs);
            }
            if !subs.is_empty() {
                env.apply_substitutions(&subs);
                break;
            }
        }
    }

    // spawn! é fire-and-forget como fork! — não retorna valor.
    // A comunicação entre parent e child é exclusivamente por canais.
    let ret_ty = Ty::Unit;

    Ok(ActionDispatch::Complete(TypedExpr {
        span: *span,
        ty: ret_ty,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Spawn {
            action_name,
            action_expr: Box::new(Spanned::new(action_expr_typed, *span)),
            args: Box::new(Spanned::new(typed_args, args_span)),
        },
    }))
}

/// Normaliza Grouping → Tuple (mesmo que action_call.rs principal).
fn normalize_grouping(typed: TypedExpr) -> TypedExpr {
    match &typed.kind {
        TypedExprKind::Grouping { inner } => {
            let inner = inner.clone();
            TypedExpr {
                ty: Ty::Tuple(vec![inner.node.ty.clone()]),
                kind: TypedExprKind::Tuple {
                    elements: vec![*inner],
                },
                span: typed.span,
                tail_pos: typed.tail_pos,
                escape: typed.escape,
            }
        }
        _ => typed,
    }
}
