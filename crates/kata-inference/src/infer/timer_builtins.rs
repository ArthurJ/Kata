//! Timer builtins — `now!()`.
//!
//! `now!()` desugara para `kata_rt_timer_now()` — clock monotônico em
//! nanossegundos. Não recebe argumentos. Retorna `Int`.
//!
//! É a base funcional que `@timer` usa internamente, mas pode ser
//! chamado isoladamente pelo usuário para medição manual.

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::Ty;
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::action_call::ActionDispatch;
use super::helpers::InferResult;

/// `now!()` — desugara para `kata_rt_timer_now`.
///
/// Valida arity 0: `now!()` não recebe argumentos.
/// Retorna `Int` (nanossegundos do clock monotônico).
pub(crate) fn infer_now_builtin(args: &Spanned<Expr>, span: &Span) -> InferResult<ActionDispatch> {
    // Valida arity 0 — args deve ser Unit (`now!()` não recebe argumentos).
    if !matches!(args.node, Expr::Unit) {
        // Tenta extrair elementos para reportar o arity encontrado.
        let found = match &args.node {
            Expr::Tuple { elements } => elements.len(),
            Expr::Grouping { inner } => match &inner.node {
                Expr::Tuple { elements } => elements.len(),
                _ => 1, // Grouping de expr única = 1 arg
            },
            _ => 1, // Expr não-tupla = 1 arg (não deveria chegar aqui)
        };
        return Err(MiddleError::ArityMismatch {
            expected: 0,
            found,
            span: args.span.into(),
        hint: None,
        });
    }

    // Constrói a expressão callee: Ident "kata_rt_timer_now" com tipo
    // função () -> Int.
    let callee = TypedExpr {
        span: *span,
        ty: Ty::Function(vec![], Box::new(Ty::int())),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_timer_now".into(),
        },
    };

    // Constrói a Closure { ffi_symbol: "kata_rt_timer_now" } — chamada
    // direta à FFI, sem args.
    let typed = TypedExpr {
        span: *span,
        ty: Ty::int(),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Caller,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, *span)),
            args: vec![],
            ffi_symbol: Some("kata_rt_timer_now".into()),
        },
    };

    Ok(ActionDispatch::Complete(typed))
}
