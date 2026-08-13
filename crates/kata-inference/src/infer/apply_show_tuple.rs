//! `show tuple` e `show ()` special cases — intercepta `show` quando o arg é
//! `Ty::Tuple` ou `Ty::Unit`.
//!
//! Tuple é estrutural (não nominal) e não registra overload de `show` no
//! DispatchTable. Este módulo gera uma Closure genérica (callee = Ident("show"),
//! ffi_symbol = None) que o monomorphizador resolve via `tuple_show.rs`,
//! sintetizando a árvore de string_concat com FieldAccess para cada elemento.
//!
//! `Ty::Unit` (`()`) também não tem overload de `show`. Retorna `TextLit("()")`
//! diretamente — não precisa de resolução no monomorphizador.
//!
//! Sem isso, `show (1, 2)` e `show ()` falham com `type.no_overload`.

use kata_ast::{Expr, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Tenta a forma `show <tuple>` ou `show ()`. Retorna `Some(Ok(..))` se o arg
/// é Tuple (gera Closure genérica para o monomorphizador resolver) ou Unit
/// (retorna `TextLit("()")` direto), `None` caso contrário.
pub(crate) fn try_show_tuple(
    func_name: &str,
    args: &[Spanned<Expr>],
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> Option<InferResult<(Ty, TypedExprKind)>> {
    if func_name != "show" || args.len() != 1 {
        return None;
    }

    let callee = &args[0];

    let typed_arg = match infer_expr(&callee.node, &callee.span, env, ctx, false) {
        Ok(t) => t,
        Err(e) => return Some(Err(e)),
    };

    // Ty::Unit → retorna TextLit("()") direto
    if matches!(&typed_arg.ty, Ty::Unit) {
        return Some(Ok((
            Ty::text(),
            TypedExprKind::TextLit { text: "()".to_string() },
        )));
    }

    if !matches!(&typed_arg.ty, Ty::Tuple(_)) {
        return None;
    }

    let arg_ty = typed_arg.ty.clone();
    let typed_arg_spanned = Spanned::new(typed_arg, callee.span);

    // Gera `show <tuple>` como Closure genérica (ffi_symbol: None).
    // O monomorphizador encontra `tuple_show.rs::rewrite_show_tuple_call`
    // e substitui pela árvore de string_concat.
    let callee_typed = TypedExpr {
        span: callee.span,
        ty: Ty::Function(vec![arg_ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "show".to_string(),
        },
    };

    Some(Ok((
        Ty::text(),
        TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee_typed, callee.span)),
            args: vec![typed_arg_spanned],
            ffi_symbol: None,
        },
    )))
}
