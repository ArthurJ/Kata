//! `repr <expr>` special case — intercepta `repr` quando chamado pelo usuário.
//!
//! `repr` é o protocolo round-tripable: igual a `show` para todos os tipos,
//! exceto Text, onde cita o argumento com aspas duplas.
//!
//! Quando o usuário escreve `repr "hello"`, este interceptador:
//! - Text concreto: gera `string_concat("\"", string_concat(arg, "\""))` inline
//! - Outro tipo concreto: delega para `show_expr` (mesma FFI)
//! - Ty::Var: gera Closure genérica `repr <arg>` com ffi_symbol: None
//!   (o monomorphizador resolve via Layer 7)

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};

use crate::typed::TypedExprKind;

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;
use super::show_synthesis_helpers::repr_expr;

/// Tenta a forma `repr <expr>`. Retorna `Some(Ok(..))` se `func_name == "repr"`
/// e há 1 arg, `None` caso contrário.
pub(crate) fn try_repr(
    func_name: &str,
    args: &[Spanned<Expr>],
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> Option<InferResult<(Ty, TypedExprKind)>> {
    if func_name != "repr" || args.len() != 1 {
        return None;
    }

    let callee = &args[0];

    let typed_arg = match infer_expr(&callee.node, &callee.span, env, ctx, false) {
        Ok(t) => t,
        Err(e) => return Some(Err(e)),
    };

    let arg_ty = typed_arg.ty.clone();
    let typed_arg_spanned = Spanned::new(typed_arg, callee.span);

    // repr_expr já faz exatamente o que precisamos:
    // - Text → cita com aspas
    // - Var → Closure genérica (ffi_symbol: None) para o monomorph resolver
    // - Outro → delega para show_expr
    let result_expr = repr_expr(typed_arg_spanned, &arg_ty);

    Some(Ok((Ty::text(), result_expr.node.kind)))
}
