//! Builtins CSP (Communicating Sequential Processes) — channel, queue.
//!
//! Extraído de `action_call.rs` — funções self-contained que lidam com
//! os builtins de canal:
//! - `channel!()` / `broadcast!()` → `ChannelCreate { Rendezvous|Broadcast, T0 }`
//! - `queue!(N)` → `ChannelCreate { Buffered(N), T0 }`
//!
//! fork!/spawn! foram extraídos para `csp_concurrency.rs`.

use std::cell::Cell;

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::Ty;

use crate::typed::{ChannelKind, TypedExpr, TypedExprKind};

use super::action_call::ActionDispatch;
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
pub(crate) fn extract_var_subs(param: &Ty, arg: &Ty, subs: &mut super::generics::Substitutions) {
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
