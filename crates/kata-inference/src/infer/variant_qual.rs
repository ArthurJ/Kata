//! Inferência de `Expr::VariantQual` — qualificação de variante sem Apply.
//!
//! Quando o usuário escreve `Enum::Variant` (sem Apply), este módulo resolve
//! a variante no EnumRegistry e produz o tipo apropriado:
//! - Enum genérico → `Ty::Generic` com type_args como `Ty::Var` (não-inferido)
//! - Enum simples → `Ty::Sum`

use kata_ast::Span;
use kata_core::ty::{PrimTy, Ty};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::InferCtx;

/// Constrói um TypedExpr literal a partir do texto bruto do valor fixo.
/// `IntLit` para Int, `FloatLit` para Float, `TextLit` para Text.
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

/// Infere o tipo de `Enum::Variant` (variante unitária sem Apply).
///
/// Retorna `Ok(Some((ty, kind)))` quando o braço foi tratado,
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
) -> Result<Option<(Ty, TypedExprKind)>, MiddleError> {
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
            // Variante constante: OK sem args constrói com valor fixo.
            if let Some(_fixed) = ctx.enum_registry.fixed_value(name, variant) {
                let tag = ctx
                    .enum_registry
                    .variant_index(name, variant)
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: format!("{}::{}", name, variant),
                        span: (*span).into(),
                    })?;
                // TODO: produzir VariantConstruct com payload = literal do fixed_value.
                // Por ora, VariantQual (sem payload) — o codegen precisaria do valor.
                // Isso é uma implementação parcial; o caso genérico + fixed_value é raro.
                let type_params = ctx
                    .enum_registry
                    .type_params_of(name)
                    .expect("is_generic true");
                let type_args: Vec<Ty> = type_params.iter().map(|p| Ty::Var(p.clone())).collect();
                let result_ty = Ty::Generic(name.clone(), type_args);
                return Ok(Some((
                    result_ty,
                    TypedExprKind::VariantQual {
                        enum_name: name.clone(),
                        variant: variant.to_string(),
                        tag,
                    },
                )));
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
            // Variante constante: OK sem args constrói com valor fixo.
            if let Some(fixed_text) = ctx.enum_registry.fixed_value(name, variant) {
                let tag = ctx
                    .enum_registry
                    .variant_index(name, variant)
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: format!("{}::{}", name, variant),
                        span: (*span).into(),
                    })?;
                let payload_ty = ctx
                    .enum_registry
                    .payload_ty(name, variant)
                    .expect("fixed_value implica payload_ty inferido");
                // Constrói TypedExpr do literal a partir do texto bruto.
                let payload = build_fixed_payload(fixed_text, payload_ty, *span);
                return Ok(Some((
                    enum_ty.clone(),
                    TypedExprKind::VariantConstruct {
                        enum_name: name.clone(),
                        variant: variant.to_string(),
                        payload: Box::new(kata_ast::Spanned::new(payload, *span)),
                        tag,
                    },
                )));
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
            )))
        }
        _ => Ok(None),
    }
}
