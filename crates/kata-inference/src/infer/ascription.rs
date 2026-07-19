//! Inferência de `TypeAscription` (`expr::Type`).
//!
//! Extraído de `expr.rs` — responsabilidade: lidar com ascription-refined
//! (validação compile-time de predicados), ascription-construção
//! (Tuple→StructConstruct), e rebaixamento de literais (Int→Float, etc.).

use kata_ast::{Expr, Span, Spanned, TypeExpr};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr_hinted};
use super::helpers::{InferResult, resolve_type_expr};

/// Infere uma `TypeAscription` — `expr::Type`.
///
/// Cenários:
/// 1. **Grouped ascription** `((expr))::Type` — barreira de hint (sem hint).
/// 2. **Ascription-refined** `5::PositiveInt` — valida predicados em compile-time.
/// 3. **Ascription-construção** `(a, b)::Pessoa` — Tuple→StructConstruct.
/// 4. **Rebaixamento** `42::Float` — IntLit→Float, etc.
pub(crate) fn infer_type_ascription(
    expr: &Spanned<Expr>,
    ty: &Spanned<TypeExpr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    _hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    let target_ty = resolve_type_expr(&ty.node, env);

    // Grouped ascription `((expr))::Type` — barreira de hint.
    // Se expr é Grouping(Grouping(inner2)), o grouping duplo bloqueia
    // a propagação do hint. Inferir inner2 sem hint (None), depois
    // validar contra target_ty normalmente.
    let (inner, _is_grouped) = match &expr.node {
        Expr::Grouping { inner: g1 } => {
            if let Expr::Grouping { inner: g2 } = &g1.node {
                // ((expr))::Type — barreira: sem hint
                let typed = infer_expr_hinted(&g2.node, &g2.span, env, ctx, false, None)?;
                (typed, true)
            } else {
                // (expr)::Type — propaga hint normalmente
                let typed =
                    infer_expr_hinted(&expr.node, &expr.span, env, ctx, false, Some(&target_ty))?;
                (typed, false)
            }
        }
        _ => {
            // expr::Type — propaga hint normalmente
            let typed =
                infer_expr_hinted(&expr.node, &expr.span, env, ctx, false, Some(&target_ty))?;
            (typed, false)
        }
    };

    // Ascription-refined — `5::PositiveInt` valida predicados
    // em compile-time. Se target é um tipo refined (StructInfo com
    // predicates) e expr é literal, avalia cada predicado via
    // const_eval. Se todos passam → TypeAscription com target_ty.
    if let Ty::Struct(ref struct_name) = target_ty
        && let Some(struct_info) = ctx.struct_registry.get(struct_name)
        && struct_info.predicates.is_some()
    {
        // Refined type — expr deve ser literal numérico.
        let is_literal = matches!(
            inner.kind,
            TypedExprKind::IntLit { .. } | TypedExprKind::FloatLit { .. }
        );
        if !is_literal {
            return Err(MiddleError::TypeMismatch {
                expected: format!(
                    "literal para ascription refined {struct_name} \
                     (use construtor para expr não-literal)"
                ),
                found: format!("{:?}", inner.kind),
                span: expr.span.into(),
            });
        }

        // Busca os predicados em refined_decls.
        let refined_decl = ctx
            .refined_decls
            .iter()
            .find(|rd| rd.name == *struct_name)
            .ok_or_else(|| MiddleError::TypeMismatch {
                expected: format!("RefinedDeclInfo para {struct_name}"),
                found: "não encontrado em refined_decls".into(),
                span: expr.span.into(),
            })?;

        // Avalia cada predicado sobre o literal.
        for (i, pred) in refined_decl.predicates.iter().enumerate() {
            match super::const_eval::const_eval_predicate(pred, expr) {
                Some(true) => {} // predicado satisfeito
                Some(false) => {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("predicado {i} de {struct_name} satisfeito"),
                        found: "predicado falhou para valor".to_string(),
                        span: expr.span.into(),
                    });
                }
                None => {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!(
                            "predicado {i} de {struct_name} avaliável em compile-time"
                        ),
                        found: "predicado muito complexo — use construtor falível".into(),
                        span: expr.span.into(),
                    });
                }
            }
        }

        // Todos os predicados passaram — produz TypeAscription.
        return Ok(TypedExpr {
            span: *span,
            ty: target_ty.clone(),
            tail_pos,
            escape: if ctx.ret_ty.is_some() {
                if tail_pos {
                    EscapeTarget::Caller
                } else {
                    EscapeTarget::Local
                }
            } else {
                EscapeTarget::Ancestor(0)
            },
            effect: Effect::Puro,
            kind: TypedExprKind::TypeAscription {
                expr: Box::new(Spanned::new(inner, expr.span)),
                target_ty,
            },
        });
    }

    // Ascription-construção — `(a, b)::Pessoa` → StructConstruct.
    // Se inner é Tuple e target é Struct, e o shape bate (mesmo nº de
    // elementos, tipos compatíveis), produz StructConstruct.
    if let Ty::Struct(ref struct_name) = target_ty
        && let TypedExprKind::Tuple { elements } = &inner.kind
        && let Some(struct_info) = ctx.struct_registry.get(struct_name)
        && !struct_info.fields.is_empty()
        && struct_info.alias_of.is_none()
    {
        // Shape check: mesmo número de elementos
        if elements.len() != struct_info.fields.len() {
            return Err(MiddleError::TypeMismatch {
                expected: format!(
                    "Struct {} with {} fields",
                    struct_name,
                    struct_info.fields.len()
                ),
                found: format!("Tuple with {} elements", elements.len()),
                span: expr.span.into(),
            });
        }
        // Verifica tipos compatíveis
        let mut shape_ok = true;
        for (elem, field) in elements.iter().zip(struct_info.fields.iter()) {
            if elem.node.ty != field.ty {
                shape_ok = false;
                break;
            }
        }
        if shape_ok {
            let values = elements
                .iter()
                .map(|e| Spanned::new(e.node.clone(), e.span))
                .collect();
            return Ok(TypedExpr {
                span: *span,
                ty: target_ty.clone(),
                tail_pos,
                escape: if ctx.ret_ty.is_some() {
                    if tail_pos {
                        EscapeTarget::Caller
                    } else {
                        EscapeTarget::Local
                    }
                } else {
                    EscapeTarget::Ancestor(0)
                },
                effect: Effect::Puro,
                kind: TypedExprKind::StructConstruct {
                    struct_name: struct_name.clone(),
                    values,
                },
            });
        }
        // Shape mismatch (tipos incompatíveis) → error
        return Err(MiddleError::TypeMismatch {
            expected: format!(
                "Struct {} fields [{}]",
                struct_name,
                struct_info
                    .fields
                    .iter()
                    .map(|f| f.ty.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            found: format!(
                "Tuple elements [{}]",
                elements
                    .iter()
                    .map(|e| e.node.ty.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            span: expr.span.into(),
        });
    }

    let rebaixa_ok = match (&inner.kind, &target_ty) {
        (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Int)) => true,
        (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Float)) => true,
        (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
        (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Float)) => true,
        (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
        (TypedExprKind::TextLit { .. }, Ty::Prim(PrimTy::Text)) => true,
        _ if inner.ty == target_ty => true,
        _ => false,
    };

    if !rebaixa_ok {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", target_ty),
            found: format!("{}", inner.ty),
            span: expr.span.into(),
        });
    }

    Ok(TypedExpr {
        span: *span,
        ty: target_ty.clone(),
        tail_pos,
        escape: if ctx.ret_ty.is_some() {
            if tail_pos {
                EscapeTarget::Caller
            } else {
                EscapeTarget::Local
            }
        } else {
            EscapeTarget::Ancestor(0)
        },
        effect: Effect::Puro,
        kind: TypedExprKind::TypeAscription {
            expr: Box::new(Spanned::new(inner, expr.span)),
            target_ty,
        },
    })
}
