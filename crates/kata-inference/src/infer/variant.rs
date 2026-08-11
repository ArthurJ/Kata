//! Resolução de variantes desqualificadas — quando um identificador como
//! `True`, `None`, ou `Vermelho` é usado sem qualificar o enum.
//!
//! `resolve_unqual_variant` busca no `EnumRegistry` para determinar se o
//! nome corresponde a uma variante unitária de exatamente 1 enum.

use kata_ast::Span;
use kata_core::ty::{PrimTy, Ty};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::InferCtx;
use super::helpers::InferResult;

/// Constrói um TypedExpr literal a partir do texto bruto do valor fixo.
fn build_fixed_payload(text: &str, ty: &Ty, span: Span) -> TypedExpr {
    match ty {
        Ty::Prim(PrimTy::Int) => TypedExpr {
            span,
            ty: Ty::int(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::IntLit {
                text: text.to_string(),
            },
        },
        Ty::Prim(PrimTy::Float) => TypedExpr {
            span,
            ty: Ty::float(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::FloatLit {
                text: text.to_string(),
            },
        },
        Ty::Prim(PrimTy::Text) => TypedExpr {
            span,
            ty: Ty::text(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::TextLit {
                text: text.to_string(),
            },
        },
        _ => TypedExpr {
            span,
            ty: ty.clone(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::IntLit {
                text: text.to_string(),
            },
        },
    }
}

/// Resolve variante desqualificada em posição de expressão.
///
/// Quando `env.lookup(name)` falha, tenta o EnumRegistry: se `name` é variante
/// unitária de exatamente 1 enum, produz `VariantQual`. Se múltiplos enums têm
/// a variante, erro de ambiguidade. Se 0, `UnboundName`. Se tem payload, erro
/// (precisa de Apply: `Ok 42`, não `Ok` sozinho).
pub(crate) fn resolve_unqual_variant(
    name: &str,
    span: &Span,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)> {
    let candidates = ctx.enum_registry.find_enums_with_variant(name);
    if candidates.is_empty() {
        return Err(MiddleError::UnboundName {
            name: name.to_string(),
            span: (*span).into(),
            suggestion: None,
        });
    }
    if candidates.len() > 1 {
        return Err(MiddleError::UnboundName {
            suggestion: None,
            name: format!(
                "variante '{name}' é ambígua — existe em: {}. Qualifique (ex: {}::{name})",
                candidates.join(", "),
                candidates[0]
            ),
            span: (*span).into(),
        });
    }
    let enum_name = candidates[0];
    // Variante constante: OK sem args constrói com valor fixo.
    if let Some(fixed_text) = ctx.enum_registry.fixed_value(enum_name, name) {
        let tag = ctx
            .enum_registry
            .variant_index(enum_name, name)
            .unwrap_or(0);
        let payload_ty = ctx
            .enum_registry
            .payload_ty(enum_name, name)
            .expect("fixed_value implica payload_ty inferido");
        let payload = build_fixed_payload(fixed_text, payload_ty, *span);
        return Ok((
            Ty::Sum(enum_name.to_string()),
            TypedExprKind::VariantConstruct {
                enum_name: enum_name.to_string(),
                variant: name.to_string(),
                payload: Box::new(kata_ast::Spanned::new(payload, *span)),
                tag,
                module_path: None,
            },
        ));
    }
    if ctx.enum_registry.payload_ty(enum_name, name).is_some() {
        return Err(MiddleError::UnboundName {
            suggestion: None,
            name: format!(
                "{enum_name}::{name} tem payload — use Apply (ex: {enum_name}::{name} valor)"
            ),
            span: (*span).into(),
        });
    }
    let tag = ctx
        .enum_registry
        .variant_index(enum_name, name)
        .unwrap_or(0);
    if ctx.enum_registry.is_generic(enum_name) {
        let type_params = ctx
            .enum_registry
            .type_params_of(enum_name)
            .expect("is_generic true");
        let type_args: Vec<Ty> = type_params.iter().map(|p| Ty::Var(p.clone())).collect();
        Ok((
            Ty::Generic(enum_name.to_string(), type_args),
            TypedExprKind::VariantQual {
                enum_name: enum_name.to_string(),
                variant: name.to_string(),
                tag,
                module_path: None,
            },
        ))
    } else {
        Ok((
            Ty::Sum(enum_name.to_string()),
            TypedExprKind::VariantQual {
                enum_name: enum_name.to_string(),
                variant: name.to_string(),
                tag,
                module_path: None,
            },
        ))
    }
}
