//! Inferência de coleções — ListLit, ArrayLit, RangeLit, ForIn, In.
//!
//! Cada função é self-contained: chama `infer_expr` para sub-expressões mas
//! tem lógica própria de tipagem. O módulo é importado por `expr.rs` que
//! despacha os arms `Expr::ListLit | ArrayLit | RangeLit | ForIn | In`.

use kata_ast::{Expr, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::interface_registry::ImplEntry;
use kata_core::ty::{Ty, TypeEnv, ty_list_to_string};
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::generics::{apply_subs, unify};
use super::helpers::InferResult;

// ── Util: extrair nome do tipo para InterfaceRegistry lookup ─────────────

/// Extrai o nome do tipo concreto para consulta ao InterfaceRegistry.
/// `Ty::List(Int)` → "List", `Ty::Prim(Text)` → "Text", etc.
fn concrete_type_name(ty: &Ty) -> Option<String> {
    match ty {
        Ty::List(_) => Some("List".into()),
        Ty::Array(_) => Some("Array".into()),
        Ty::Range(_) => Some("Range".into()),
        Ty::Prim(kata_core::ty::PrimTy::Int) => Some("Int".into()),
        Ty::Prim(kata_core::ty::PrimTy::Float) => Some("Float".into()),
        Ty::Prim(kata_core::ty::PrimTy::Text) => Some("Text".into()),
        Ty::Prim(kata_core::ty::PrimTy::Rational) => Some("Rational".into()),
        Ty::Struct(name) => Some(name.clone()),
        Ty::Sum(name) => Some(name.clone()),
        Ty::Generic(name, _) => Some(name.clone()),
        _ => None,
    }
}

/// Busca o `ImplEntry` de `type_name` para `iface_name` no InterfaceRegistry.
fn find_impl<'a>(ctx: &'a InferCtx, type_name: &str, iface_name: &str) -> Option<&'a ImplEntry> {
    ctx.interface_registry
        .get_impls_for_interface(iface_name)
        .into_iter()
        .find(|e| e.type_name == type_name)
}

/// Extrai o tipo do elemento A de um tipo iterável, consultando ITERABLE.
///
/// O método `next` de ITERABLE tem assinatura `Self => Optional(A)`.
/// Unificando `iterable_ty` com `Self` (params[0]), obtemos `A` via substitution.
/// Aplicando subs no retorno `Optional(A)`, extraímos `A`.
fn extract_iter_elem_ty(ctx: &InferCtx, iterable_ty: &Ty, span: &Span) -> InferResult<Ty> {
    let type_name = concrete_type_name(iterable_ty).ok_or_else(|| MiddleError::TypeMismatch {
        expected: "tipo iterável (implementa ITERABLE)".into(),
        found: format!("{iterable_ty}"),
        span: (*span).into(),
    })?;

    let entry =
        find_impl(ctx, &type_name, "ITERABLE").ok_or_else(|| MiddleError::TypeMismatch {
            expected: format!("tipo que implementa ITERABLE ({type_name} não implementa)"),
            found: format!("{iterable_ty}"),
            span: (*span).into(),
        })?;

    // Método `next` tem params = [Self] e ret = Optional(A).
    let next_method = entry
        .methods
        .iter()
        .find(|m| m.name == "next")
        .ok_or_else(|| MiddleError::TypeMismatch {
            expected: "método `next` em ITERABLE".into(),
            found: format!("{} implementa ITERABLE sem `next`", entry.type_name),
            span: (*span).into(),
        })?;

    // Unifica iterable_ty com params[0] (Self) usando type_params do impl.
    let mut subs = std::collections::HashMap::new();
    unify(
        &next_method.params,
        std::slice::from_ref(iterable_ty),
        &entry.type_params,
        &mut subs,
    )
    .map_err(|_| MiddleError::TypeMismatch {
        expected: ty_list_to_string(&next_method.params),
        found: format!("{iterable_ty}"),
        span: (*span).into(),
    })?;

    // Aplica subs no retorno (Optional(A)) e extrai A.
    let concrete_ret = apply_subs(&next_method.ret, &subs);
    match &concrete_ret {
        Ty::Generic(name, args) if name == "Optional" && !args.is_empty() => Ok(args[0].clone()),
        _ => Err(MiddleError::TypeMismatch {
            expected: "Optional(A) como retorno de next".into(),
            found: format!("{concrete_ret}"),
            span: (*span).into(),
        }),
    }
}

// ── ListLit ──────────────────────────────────────────────────────────────

/// `[1 2 3]` → Ty::List(elem_ty) onde elem_ty é unificado de todos elementos.
/// Se `hint` for `Some(Ty::List(elem))` e a lista estiver vazia, usa `elem`
/// como tipo do elemento (inferência bidirectional).
pub(crate) fn infer_list_lit(
    elements: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    let mut typed_elements = Vec::with_capacity(elements.len());
    let mut elem_ty: Option<Ty> = None;

    for elem in elements {
        let typed = infer_expr(&elem.node, &elem.span, env, ctx, false)?;
        match &elem_ty {
            None => elem_ty = Some(typed.ty.clone()),
            Some(existing) => {
                if &typed.ty != existing {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("{existing}"),
                        found: format!("{}", typed.ty),
                        span: elem.span.into(),
                    });
                }
            }
        }
        typed_elements.push(Spanned::new(typed, elem.span));
    }

    // Lista vazia: se hint for Ty::List(elem), usa elem do hint (inferência
    // bidirectional). Senão, List(InferVar(0)) — tipo resolvido pelo uso.
    let list_ty = if elements.is_empty() {
        if let Some(Ty::List(hint_elem)) = hint {
            Ty::List(Box::new((**hint_elem).clone()))
        } else {
            Ty::List(Box::new(elem_ty.unwrap_or(Ty::InferVar(0))))
        }
    } else {
        Ty::List(Box::new(elem_ty.unwrap_or(Ty::InferVar(0))))
    };

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Ancestor(0)
    };

    Ok(TypedExpr {
        span: *span,
        ty: list_ty,
        tail_pos,
        escape,
        effect: Effect::Puro,
        kind: TypedExprKind::ListLit {
            elements: typed_elements,
        },
    })
}

// ── ArrayLit ─────────────────────────────────────────────────────────────

/// `{1 2 3}` → Ty::Array(elem_ty).
pub(crate) fn infer_array_lit(
    elements: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let mut typed_elements = Vec::with_capacity(elements.len());
    let mut elem_ty: Option<Ty> = None;

    for elem in elements {
        let typed = infer_expr(&elem.node, &elem.span, env, ctx, false)?;
        match &elem_ty {
            None => elem_ty = Some(typed.ty.clone()),
            Some(existing) => {
                if &typed.ty != existing {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("{existing}"),
                        found: format!("{}", typed.ty),
                        span: elem.span.into(),
                    });
                }
            }
        }
        typed_elements.push(Spanned::new(typed, elem.span));
    }

    let array_ty = Ty::Array(Box::new(elem_ty.unwrap_or(Ty::InferVar(0))));

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Ancestor(0)
    };

    Ok(TypedExpr {
        span: *span,
        ty: array_ty,
        tail_pos,
        escape,
        effect: Effect::Puro,
        kind: TypedExprKind::ArrayLit {
            elements: typed_elements,
        },
    })
}

// ── RangeLit ─────────────────────────────────────────────────────────────

/// `[a..s..b]` ou `[a..s..=b]` → Ty::Range(A) onde A = tipo de start/step/end.
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_range_lit(
    start: &Spanned<Expr>,
    step: &Spanned<Expr>,
    end: &Spanned<Expr>,
    inclusive: bool,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let typed_start = infer_expr(&start.node, &start.span, env, ctx, false)?;
    let typed_step = infer_expr(&step.node, &step.span, env, ctx, false)?;
    let typed_end = infer_expr(&end.node, &end.span, env, ctx, false)?;

    // start, step, end devem ser do mesmo tipo.
    let elem_ty = typed_start.ty.clone();
    if typed_step.ty != elem_ty {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{elem_ty}"),
            found: format!("{}", typed_step.ty),
            span: step.span.into(),
        });
    }
    if typed_end.ty != elem_ty {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{elem_ty}"),
            found: format!("{}", typed_end.ty),
            span: end.span.into(),
        });
    }

    let range_ty = Ty::Range(Box::new(elem_ty.clone()));

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Ancestor(0)
    };

    Ok(TypedExpr {
        span: *span,
        ty: range_ty,
        tail_pos,
        escape,
        effect: Effect::Puro,
        kind: TypedExprKind::RangeLit {
            start: Box::new(Spanned::new(typed_start, start.span)),
            step: Box::new(Spanned::new(typed_step, step.span)),
            end: Box::new(Spanned::new(typed_end, end.span)),
            inclusive,
            elem_ty,
        },
    })
}

// ── ForIn ────────────────────────────────────────────────────────────────

/// `for x in colecao` → Unit. Define `x: A` no escopo do body.
pub(crate) fn infer_for_in(
    var_name: &str,
    iterable: &Spanned<Expr>,
    body: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    _tail_pos: bool,
) -> InferResult<TypedExpr> {
    let typed_iterable = infer_expr(&iterable.node, &iterable.span, env, ctx, false)?;

    // Extrai A via InterfaceRegistry lookup.
    let var_ty = extract_iter_elem_ty(ctx, &typed_iterable.ty, span)?;

    // Cria escopo filho para o body, define x: A.
    let mut body_env = env.push_scope();
    body_env.define(var_name, var_ty.clone());

    // ForIn é como loop — in_loop = true para break/continue.
    let loop_ctx = InferCtx {
        table: ctx.table,
        enum_registry: ctx.enum_registry,
        struct_registry: ctx.struct_registry,
        refined_decls: ctx.refined_decls,
        interface_registry: ctx.interface_registry,
        ret_ty: ctx.ret_ty,
        in_loop: true,
    };

    let mut typed_body = Vec::new();
    for expr in body {
        let typed = infer_expr(&expr.node, &expr.span, &mut body_env, &loop_ctx, false)?;
        typed_body.push(Spanned::new(typed, expr.span));
    }

    let escape = if ctx.ret_ty.is_some() {
        EscapeTarget::Local
    } else {
        EscapeTarget::Ancestor(0)
    };

    Ok(TypedExpr {
        span: *span,
        ty: Ty::Unit,
        tail_pos: false,
        escape,
        effect: Effect::Puro,
        kind: TypedExprKind::ForIn {
            var_name: var_name.to_string(),
            var_ty,
            iterable: Box::new(Spanned::new(typed_iterable, iterable.span)),
            body: typed_body,
        },
    })
}

// ── In (membership) ──────────────────────────────────────────────────────

/// `x in coll` → Boolean. Verifica que coll implementa CONTAINS.
pub(crate) fn infer_in(
    item: &Spanned<Expr>,
    collection: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let typed_collection = infer_expr(&collection.node, &collection.span, env, ctx, false)?;
    let typed_item = infer_expr(&item.node, &item.span, env, ctx, false)?;

    // Verifica que a coleção implementa CONTAINS.
    let coll_type_name =
        concrete_type_name(&typed_collection.ty).ok_or_else(|| MiddleError::TypeMismatch {
            expected: "tipo que implementa CONTAINS".into(),
            found: format!("{}", typed_collection.ty),
            span: (*span).into(),
        })?;

    let entry =
        find_impl(ctx, &coll_type_name, "CONTAINS").ok_or_else(|| MiddleError::TypeMismatch {
            expected: format!("tipo que implementa CONTAINS ({coll_type_name} não implementa)"),
            found: format!("{}", typed_collection.ty),
            span: (*span).into(),
        })?;

    // Verifica que o método `contains` aceita o tipo do item.
    let contains_method = entry
        .methods
        .iter()
        .find(|m| m.name == "contains")
        .ok_or_else(|| MiddleError::TypeMismatch {
            expected: "método `contains` em CONTAINS".into(),
            found: format!("{} implementa CONTAINS sem `contains`", entry.type_name),
            span: (*span).into(),
        })?;

    // contains tem params = [Self, A] e ret = Boolean.
    // Unifica Self com collection e A com item.
    if contains_method.params.len() < 2 {
        return Err(MiddleError::TypeMismatch {
            expected: "contains com 2 params (Self, A)".into(),
            found: format!("{} params", contains_method.params.len()),
            span: (*span).into(),
        });
    }

    let mut subs = std::collections::HashMap::new();
    unify(
        &contains_method.params,
        &[typed_collection.ty.clone(), typed_item.ty.clone()],
        &entry.type_params,
        &mut subs,
    )
    .map_err(|_| MiddleError::TypeMismatch {
        expected: format!("{}", &contains_method.params[1]),
        found: format!("{}", typed_item.ty),
        span: item.span.into(),
    })?;

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Ancestor(0)
    };

    Ok(TypedExpr {
        span: *span,
        ty: Ty::Sum("Boolean".into()),
        tail_pos,
        escape,
        effect: Effect::Puro,
        kind: TypedExprKind::In {
            item: Box::new(Spanned::new(typed_item, item.span)),
            collection: Box::new(Spanned::new(typed_collection, collection.span)),
        },
    })
}
