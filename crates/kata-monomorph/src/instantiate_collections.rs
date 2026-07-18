//! Instanciação de arms de collections no `instantiate_kind`.
//!
//! Extraído de `instantiate.rs` para separar a recursão de collections
//! (ListLit, ArrayLit, RangeLit, ForIn, In, Map, Filter, Fold, FusedStream)
//! da recursão base (identifiers, literals, lambda, match, etc.).
//!
//! Cada arm chama `instantiate_typed_expr` (recursão para o parent) e
//! `apply_subs` nos tipos das coleções.

use kata_ast::Spanned;
use kata_inference::{FusedStage, Substitutions, TypedExprKind, apply_subs};

use crate::instantiate::instantiate_typed_expr;

/// Instancia arms de collections no `instantiate_kind`.
///
/// Retorna `Some(kind)` se o arm é de collection, `None` caso contrário
/// (caller continua o match).
pub(crate) fn instantiate_collections(
    kind: &TypedExprKind,
    subs: &Substitutions,
) -> Option<TypedExprKind> {
    match kind {
        // ── Coleções: instanciar sub-expressões ──
        TypedExprKind::ListLit { elements } => Some(TypedExprKind::ListLit {
            elements: elements
                .iter()
                .map(|e| Spanned::new(instantiate_typed_expr(&e.node, subs), e.span))
                .collect(),
        }),
        TypedExprKind::ArrayLit { elements } => Some(TypedExprKind::ArrayLit {
            elements: elements
                .iter()
                .map(|e| Spanned::new(instantiate_typed_expr(&e.node, subs), e.span))
                .collect(),
        }),
        TypedExprKind::RangeLit {
            start,
            step,
            end,
            inclusive,
            elem_ty,
        } => Some(TypedExprKind::RangeLit {
            start: Box::new(Spanned::new(
                instantiate_typed_expr(&start.node, subs),
                start.span,
            )),
            step: Box::new(Spanned::new(
                instantiate_typed_expr(&step.node, subs),
                step.span,
            )),
            end: Box::new(Spanned::new(
                instantiate_typed_expr(&end.node, subs),
                end.span,
            )),
            inclusive: *inclusive,
            elem_ty: apply_subs(elem_ty, subs),
        }),
        TypedExprKind::ForIn {
            var_name,
            var_ty,
            iterable,
            body,
        } => Some(TypedExprKind::ForIn {
            var_name: var_name.clone(),
            var_ty: apply_subs(var_ty, subs),
            iterable: Box::new(Spanned::new(
                instantiate_typed_expr(&iterable.node, subs),
                iterable.span,
            )),
            body: body
                .iter()
                .map(|s| Spanned::new(instantiate_typed_expr(&s.node, subs), s.span))
                .collect(),
        }),
        TypedExprKind::In { item, collection } => Some(TypedExprKind::In {
            item: Box::new(Spanned::new(
                instantiate_typed_expr(&item.node, subs),
                item.span,
            )),
            collection: Box::new(Spanned::new(
                instantiate_typed_expr(&collection.node, subs),
                collection.span,
            )),
        }),

        // ── map/filter/fold: instanciar sub-exprs + Ty ──
        TypedExprKind::Map {
            callback,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => Some(TypedExprKind::Map {
            callback: Box::new(Spanned::new(
                instantiate_typed_expr(&callback.node, subs),
                callback.span,
            )),
            collection: Box::new(Spanned::new(
                instantiate_typed_expr(&collection.node, subs),
                collection.span,
            )),
            coll_ty: apply_subs(coll_ty, subs),
            elem_ty: apply_subs(elem_ty, subs),
            ret_ty: apply_subs(ret_ty, subs),
        }),

        TypedExprKind::Filter {
            callback,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => Some(TypedExprKind::Filter {
            callback: Box::new(Spanned::new(
                instantiate_typed_expr(&callback.node, subs),
                callback.span,
            )),
            collection: Box::new(Spanned::new(
                instantiate_typed_expr(&collection.node, subs),
                collection.span,
            )),
            coll_ty: apply_subs(coll_ty, subs),
            elem_ty: apply_subs(elem_ty, subs),
            ret_ty: apply_subs(ret_ty, subs),
        }),

        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => Some(TypedExprKind::Fold {
            callback: Box::new(Spanned::new(
                instantiate_typed_expr(&callback.node, subs),
                callback.span,
            )),
            initial: Box::new(Spanned::new(
                instantiate_typed_expr(&initial.node, subs),
                initial.span,
            )),
            collection: Box::new(Spanned::new(
                instantiate_typed_expr(&collection.node, subs),
                collection.span,
            )),
            coll_ty: apply_subs(coll_ty, subs),
            elem_ty: apply_subs(elem_ty, subs),
            ret_ty: apply_subs(ret_ty, subs),
        }),

        // ── FusedStream: instanciar stages + source + Ty ──
        TypedExprKind::FusedStream {
            stages,
            source,
            coll_ty,
            source_elem_ty,
            result_elem_ty,
            ret_ty,
        } => {
            let new_stages = stages
                .iter()
                .map(|stage| match stage {
                    FusedStage::Filter {
                        callback,
                        input_elem_ty,
                    } => FusedStage::Filter {
                        callback: Box::new(Spanned::new(
                            instantiate_typed_expr(&callback.node, subs),
                            callback.span,
                        )),
                        input_elem_ty: apply_subs(input_elem_ty, subs),
                    },
                    FusedStage::Map {
                        callback,
                        input_elem_ty,
                        output_elem_ty,
                    } => FusedStage::Map {
                        callback: Box::new(Spanned::new(
                            instantiate_typed_expr(&callback.node, subs),
                            callback.span,
                        )),
                        input_elem_ty: apply_subs(input_elem_ty, subs),
                        output_elem_ty: apply_subs(output_elem_ty, subs),
                    },
                })
                .collect();
            Some(TypedExprKind::FusedStream {
                stages: new_stages,
                source: Box::new(Spanned::new(
                    instantiate_typed_expr(&source.node, subs),
                    source.span,
                )),
                coll_ty: apply_subs(coll_ty, subs),
                source_elem_ty: apply_subs(source_elem_ty, subs),
                result_elem_ty: apply_subs(result_elem_ty, subs),
                ret_ty: apply_subs(ret_ty, subs),
            })
        }

        _ => None,
    }
}
