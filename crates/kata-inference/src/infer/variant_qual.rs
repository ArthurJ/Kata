//! Inferência de `Expr::VariantQual` — qualificação de variante sem Apply.
//!
//! Quando o usuário escreve `Enum::Variant` (sem Apply), este módulo resolve
//! a variante no EnumRegistry e produz o tipo apropriado:
//! - Enum genérico → `Ty::Generic` com type_args como `Ty::Var` (não-inferido)
//! - Enum simples → `Ty::Sum`

use kata_ast::Span;
use kata_core::ty::Ty;
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExprKind};

use super::expr::InferCtx;

/// Infere o tipo de `Enum::Variant` (variante unitária sem Apply).
///
/// Retorna `Ok(Some((ty, kind, effect)))` quando o braço foi tratado,
/// `Ok(None)` quando o enum não é `Ty::Sum` (fallback para o caller).
///
/// O caller deve passar o `enum_ty` já resolvido do `TypeEnv`.
#[allow(dead_code)]
pub(crate) fn infer_variant_qual(
    _enum_name: &str,
    variant: &str,
    enum_ty: &Ty,
    span: &Span,
    ctx: &InferCtx,
) -> Result<Option<(Ty, TypedExprKind, Effect)>, MiddleError> {
    match enum_ty {
        // Enum genérico no TypeEnv como Ty::Sum, mas o EnumRegistry
        // marca como genérico. Para variantes unitárias (Optional::None),
        // produz Ty::Generic com type_args não-inferidos (Ty::Var).
        Ty::Sum(name) if ctx.enum_registry.is_generic(name) => {
            if !ctx.enum_registry.is_variant(name, variant) {
                return Err(MiddleError::UnboundName {
                    name: format!("{}::{}", name, variant),
                    span: (*span).into(),
                });
            }
            if ctx.enum_registry.payload_ty(name, variant).is_some() {
                return Err(MiddleError::TypeMismatch {
                    expected: "aplicação de argumento (Result::Ok valor)".into(),
                    found: format!("{}::{} tem payload — use Apply", name, variant),
                    span: (*span).into(),
                });
            }
            let tag = ctx
                .enum_registry
                .variant_index(name, variant)
                .ok_or_else(|| MiddleError::UnboundName {
                    name: format!("{}::{}", name, variant),
                    span: (*span).into(),
                })?;
            // Para variantes unitárias de enum genérico (Optional::None),
            // não há arg para inferir os type params. Produz Ty::Generic
            // com type_args como Ty::Var (não-inferido).
            let type_params = ctx
                .enum_registry
                .type_params_of(name)
                .expect("is_generic true");
            let type_args: Vec<Ty> = type_params.iter().map(|p| Ty::Var(p.clone())).collect();
            let result_ty = Ty::Generic(name.clone(), type_args);
            Ok(Some((
                result_ty,
                TypedExprKind::VariantQual {
                    enum_name: name.clone(),
                    variant: variant.to_string(),
                    tag,
                },
                Effect::Puro,
            )))
        }
        Ty::Sum(name) => {
            // Verifica que a variante existe.
            if !ctx.enum_registry.is_variant(name, variant) {
                return Err(MiddleError::UnboundName {
                    name: format!("{}::{}", name, variant),
                    span: (*span).into(),
                });
            }
            // VariantQual sem Apply só é válido para variantes unitárias.
            // Variantes com payload exigem Apply (Result::Ok 42).
            if ctx.enum_registry.payload_ty(name, variant).is_some() {
                return Err(MiddleError::TypeMismatch {
                    expected: "aplicação de argumento (Result::Ok valor)".into(),
                    found: format!("{}::{} tem payload — use Apply", name, variant),
                    span: (*span).into(),
                });
            }
            let tag = ctx
                .enum_registry
                .variant_index(name, variant)
                .ok_or_else(|| MiddleError::UnboundName {
                    name: format!("{}::{}", name, variant),
                    span: (*span).into(),
                })?;
            Ok(Some((
                enum_ty.clone(),
                TypedExprKind::VariantQual {
                    enum_name: name.clone(),
                    variant: variant.to_string(),
                    tag,
                },
                Effect::Puro,
            )))
        }
        _ => Ok(None),
    }
}
