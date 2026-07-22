//! Inferência de DictLit e SetLit (Fio 13).
//!
//! `DictLit` — `{"k": v ...}` → `Ty::Dict(K, V)`.
//! `SetLit` — `{|1 2 3|}` → `Ty::Set(T)`.
//!
//! Para ambos, o tipo do elemento (key para Dict, elem para Set) deve
//! implementar `HASHABLE` — verificado via `InterfaceRegistry`.

use kata_ast::{Expr, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty, TypeEnv};

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Extrai o nome de um tipo concreto para consulta ao InterfaceRegistry.
/// Reutiliza a mesma lógica de `collections.rs::concrete_type_name`.
fn concrete_type_name(ty: &Ty) -> Option<String> {
    match ty {
        Ty::List(_) => Some("List".into()),
        Ty::Array(_) => Some("Array".into()),
        Ty::Range(_) => Some("Range".into()),
        Ty::Set(_) => Some("Set".into()),
        Ty::Dict(_, _) => Some("Dict".into()),
        Ty::Prim(PrimTy::Int) => Some("Int".into()),
        Ty::Prim(PrimTy::Float) => Some("Float".into()),
        Ty::Prim(PrimTy::Text) => Some("Text".into()),
        Ty::Prim(PrimTy::Rational) => Some("Rational".into()),
        Ty::Struct(name) => Some(name.clone()),
        Ty::Sum(name) => Some(name.clone()),
        Ty::Generic(name, _) => Some(name.clone()),
        _ => None,
    }
}

/// Verifica que `key_ty` implementa `HASHABLE` no InterfaceRegistry.
fn check_hashable(ctx: &InferCtx, key_ty: &Ty, span: &Span) -> InferResult<()> {
    let type_name =
        concrete_type_name(key_ty).ok_or_else(|| kata_diagnostics::MiddleError::TypeMismatch {
            expected: "tipo que implementa HASHABLE".into(),
            found: format!("{key_ty}"),
            span: (*span).into(),
        })?;

    if ctx
        .interface_registry
        .type_implements(&type_name, "HASHABLE")
    {
        Ok(())
    } else {
        Err(kata_diagnostics::MiddleError::TypeMismatch {
            expected: format!("tipo que implementa HASHABLE ({type_name} não implementa HASHABLE)"),
            found: format!("{key_ty}"),
            span: (*span).into(),
        })
    }
}

// ── DictLit ──────────────────────────────────────────────────────────────

/// `{"k": v ...}` → `Ty::Dict(K, V)`.
///
/// Infere cada key e value. Unifica todos key types (match exato para v1)
/// e todos value types. Verifica que `K` implementa `HASHABLE`.
/// Dict vazio: `K` e `V` são `Ty::InferVar(0)`.
pub(crate) fn infer_dict_lit(
    entries: &[(Spanned<Expr>, Spanned<Expr>)],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let mut typed_entries = Vec::with_capacity(entries.len());
    let mut key_ty: Option<Ty> = None;
    let mut value_ty: Option<Ty> = None;

    for (key_expr, val_expr) in entries {
        let typed_key = infer_expr(&key_expr.node, &key_expr.span, env, ctx, false)?;
        let typed_val = infer_expr(&val_expr.node, &val_expr.span, env, ctx, false)?;

        // Unifica key types (match exato para v1).
        match &key_ty {
            None => key_ty = Some(typed_key.ty.clone()),
            Some(existing) => {
                if &typed_key.ty != existing {
                    return Err(kata_diagnostics::MiddleError::TypeMismatch {
                        expected: format!("{existing}"),
                        found: format!("{}", typed_key.ty),
                        span: key_expr.span.into(),
                    });
                }
            }
        }

        // Unifica value types (match exato para v1).
        match &value_ty {
            None => value_ty = Some(typed_val.ty.clone()),
            Some(existing) => {
                if &typed_val.ty != existing {
                    return Err(kata_diagnostics::MiddleError::TypeMismatch {
                        expected: format!("{existing}"),
                        found: format!("{}", typed_val.ty),
                        span: val_expr.span.into(),
                    });
                }
            }
        }

        typed_entries.push((
            Spanned::new(typed_key, key_expr.span),
            Spanned::new(typed_val, val_expr.span),
        ));
    }

    let final_key_ty = key_ty.unwrap_or(Ty::InferVar(0));
    let final_value_ty = value_ty.unwrap_or(Ty::InferVar(0));

    // Verifica HASHABLE no key type (só se não for InferVar — empty dict).
    if !entries.is_empty() {
        check_hashable(ctx, &final_key_ty, span)?;
    }

    let dict_ty = Ty::Dict(
        Box::new(final_key_ty.clone()),
        Box::new(final_value_ty.clone()),
    );

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Caller
    };

    Ok(TypedExpr {
        span: *span,
        ty: dict_ty,
        tail_pos,
        escape,
        effect: Effect::Puro,
        kind: TypedExprKind::DictLit {
            entries: typed_entries,
            key_ty: final_key_ty,
            value_ty: final_value_ty,
        },
    })
}

// ── SetLit ───────────────────────────────────────────────────────────────

/// `{|1 2 3|}` → `Ty::Set(T)`.
///
/// Infere cada elemento. Unifica todos element types (match exato).
/// Verifica que `T` implementa `HASHABLE`.
/// Set vazio: `T` é `Ty::InferVar(0)`.
pub(crate) fn infer_set_lit(
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
                    return Err(kata_diagnostics::MiddleError::TypeMismatch {
                        expected: format!("{existing}"),
                        found: format!("{}", typed.ty),
                        span: elem.span.into(),
                    });
                }
            }
        }
        typed_elements.push(Spanned::new(typed, elem.span));
    }

    let final_elem_ty = elem_ty.unwrap_or(Ty::InferVar(0));

    // Verifica HASHABLE no elem type (só se não for InferVar — empty set).
    if !elements.is_empty() {
        check_hashable(ctx, &final_elem_ty, span)?;
    }

    let set_ty = Ty::Set(Box::new(final_elem_ty.clone()));

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Caller
    };

    Ok(TypedExpr {
        span: *span,
        ty: set_ty,
        tail_pos,
        escape,
        effect: Effect::Puro,
        kind: TypedExprKind::SetLit {
            elements: typed_elements,
            elem_ty: final_elem_ty,
        },
    })
}
