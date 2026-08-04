//! `len tuple` special case — compile-time len of a tuple literal.
//!
//! When `func_name == "len" && args.len() == 1`, we may be able to compute
//! the length at compile time (tuple arity). For non-tuple args, falls back
//! to dispatch via COUNTABLE (match_score, generic unify, refines).

use kata_ast::{Expr, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use std::collections::HashMap;

use crate::typed::{TypedExpr, TypedExprKind};

use super::apply_dispatch::try_refines_fallback;
use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Tenta a forma `len <tuple>`. Retorna `Some(Ok(..))` se dispatch sucede
/// (tuple com compile-time length, ou dispatch normal do COUNTABLE),
/// `Some(Err(..))` se falha com erro definitivo, e `None` se `func_name`
/// não é `"len"` ou `args.len() != 1`.
pub(crate) fn try_len_tuple(
    func_name: &str,
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> Option<InferResult<(Ty, TypedExprKind)>> {
    if func_name != "len" || args.len() != 1 {
        return None;
    }

    let callee = &args[0];

    let typed_arg = match infer_expr(&callee.node, &callee.span, env, ctx, false) {
        Ok(t) => t,
        Err(e) => return Some(Err(e)),
    };

    if let Ty::Tuple(elements) = &typed_arg.ty {
        return Some(Ok((
            Ty::int(),
            TypedExprKind::IntLit {
                text: elements.len().to_string(),
            },
        )));
    }

    // Não é Tuple — cai para o dispatch normal (COUNTABLE) abaixo.
    // Reusar o typed_arg já inferido para evitar dupla inferência.
    let typed_args = vec![Spanned::new(typed_arg, callee.span)];
    let arg_types: Vec<Ty> = typed_args.iter().map(|t| t.node.ty.clone()).collect();

    // Tenta dispatch normal (match_score).
    if let Ok(overload) = ctx
        .table
        .resolve(func_name, &arg_types, ctx.interface_registry)
    {
        let expanded_ret = super::apply::expand_ret(&overload.ret, ctx);
        let callee_ty = Ty::Function(overload.params.clone(), Box::new(expanded_ret.clone()));
        let callee_typed = TypedExpr {
            span: callee.span,
            ty: callee_ty,
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind: TypedExprKind::Ident {
                name: func_name.to_string(),
            },
        };
        return Some(Ok((
            expanded_ret,
            TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee_typed, callee.span)),
                args: typed_args,
                ffi_symbol: overload.ffi_symbol,
            },
        )));
    }

    // Tenta caminho genérico: overloads com type_params não-vazio.
    if let Some(overloads) = ctx.table.get_overloads(func_name) {
        for oi in overloads
            .iter()
            .filter(|oi| oi.params.len() == arg_types.len() && !oi.type_params.is_empty())
        {
            let mut subs: super::generics::Substitutions = HashMap::new();
            if super::generics::unify(&oi.params, &arg_types, &oi.type_params, &mut subs).is_ok() {
                let concrete_ret = super::generics::apply_subs(&oi.ret, &subs);
                let expanded_ret = super::apply::expand_ret(&concrete_ret, ctx);
                let callee_ty = Ty::Function(oi.params.clone(), Box::new(expanded_ret.clone()));
                let callee_typed = TypedExpr {
                    span: callee.span,
                    ty: callee_ty,
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    kind: TypedExprKind::Ident {
                        name: func_name.to_string(),
                    },
                };
                if let Err(e) = super::apply::reject_action_arg_for_pure_fn(oi, &typed_args, span) {
                    return Some(Err(e));
                }
                return Some(Ok((
                    expanded_ret,
                    TypedExprKind::Closure {
                        callee: Box::new(Spanned::new(callee_typed, callee.span)),
                        args: typed_args,
                        ffi_symbol: oi.ffi_symbol.clone(),
                    },
                )));
            }
        }
    }

    // Caminho genérico falhou. Tentar fallback refines (D1 do PRD-refines):
    // se algum arg é tipo refined com delegação refines, substituir pelo
    // tipo base e retentar o dispatch. O refined é alias do base no layout
    // — o codegen não precisa de conversão de bits, mas precisa do tipo
    // base correto para mapear F64/I64 no Cranelift.
    if let Some((fallback_arg_types, fallback_overload)) =
        try_refines_fallback(func_name, &arg_types, ctx)
    {
        // Converter typed_args para o tipo base onde o fallback substituiu.
        // Isto é um no-op em runtime (mesmos bits), mas garante que o codegen
        // use o Cranelift type correto (F64 para Float, I64 para Int).
        let converted_args: Vec<Spanned<TypedExpr>> = typed_args
            .iter()
            .zip(fallback_arg_types.iter())
            .map(|(arg, fallback_ty)| {
                if arg.node.ty != *fallback_ty {
                    Spanned::new(
                        TypedExpr {
                            span: arg.span,
                            ty: fallback_ty.clone(),
                            tail_pos: arg.node.tail_pos,
                            escape: arg.node.escape,
                            kind: TypedExprKind::TypeAscription {
                                expr: Box::new(arg.clone()),
                                target_ty: fallback_ty.clone(),
                                pending_predicates: Vec::new(),
                            },
                        },
                        arg.span,
                    )
                } else {
                    arg.clone()
                }
            })
            .collect();
        let callee_ty = Ty::Function(
            fallback_overload.params.clone(),
            Box::new(super::apply::expand_ret(&fallback_overload.ret, ctx)),
        );
        let callee_typed = TypedExpr {
            span: callee.span,
            ty: callee_ty,
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind: TypedExprKind::Ident {
                name: func_name.to_string(),
            },
        };
        return Some(Ok((
            super::apply::expand_ret(&fallback_overload.ret, ctx),
            TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee_typed, callee.span)),
                args: converted_args,
                ffi_symbol: fallback_overload.ffi_symbol,
            },
        )));
    }

    Some(Err(MiddleError::NoOverload {
        name: func_name.to_string(),
        span: (*span).into(),
    }))
}
