//! Inferência de `match` — pattern matching com verificação de exaustividade.
//!
//! Processa scrutinee + arms, casa padrões, infere guards e bodies,
//! verifica uniformidade de tipo entre braços e exaustividade.

use kata_ast::{Expr, MatchArm, Span, Spanned};
use kata_core::dispatch::DispatchTable;
use kata_core::enum_registry::EnumRegistry;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::patterns;
use crate::typed::{Effect, TypedExprKind, TypedMatchArm, TypedPattern};

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Infere um `match` — pattern matching com verificação de exaustividade.
pub(crate) fn infer_match(
    scrutinee: &Spanned<Expr>,
    arms: &[MatchArm],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    // Infere o scrutinee.
    let typed_scrutinee = infer_expr(&scrutinee.node, &scrutinee.span, env, ctx, false)?;
    let scrutinee_ty = typed_scrutinee.ty.clone();

    // Processa cada braço.
    let mut typed_arms: Vec<TypedMatchArm> = Vec::with_capacity(arms.len());
    let mut match_ret_ty: Option<Ty> = None;
    let mut covered_variants: Vec<String> = Vec::new();
    let mut has_otherwise = false;

    for arm in arms {
        // Cria escopo filho para bindings do pattern.
        let mut arm_env = env.push_scope();

        let typed_pattern = if let Some(pat) = &arm.pattern {
            let typed_pat =
                patterns::check_pattern(pat, &scrutinee_ty, ctx.enum_registry, &mut arm_env)?;
            // Coleta variantes cobertas para exaustividade.
            if let TypedPattern::Variant { variant, .. } = &typed_pat.node {
                covered_variants.push(variant.clone());
            }
            // Ident e Wildcard cobrem qualquer valor — contam como fallback.
            if matches!(
                &typed_pat.node,
                TypedPattern::Ident { .. } | TypedPattern::Wildcard
            ) {
                has_otherwise = true;
            }
            Some(typed_pat)
        } else {
            // otherwise — pattern None.
            has_otherwise = true;
            None
        };

        // Infere guard (se houver).
        let typed_guard = if let Some(guard_expr) = &arm.guard {
            let guard_typed =
                infer_expr(&guard_expr.node, &guard_expr.span, &mut arm_env, ctx, false)?;
            if guard_typed.ty != Ty::boolean() {
                return Err(MiddleError::TypeMismatch {
                    expected: "Boolean".into(),
                    found: format!("{:?}", guard_typed.ty),
                    span: guard_expr.span.into(),
                });
            }
            Some(Spanned::new(guard_typed, guard_expr.span))
        } else {
            None
        };

        // Infere body do braço.
        let typed_body = infer_expr(&arm.body.node, &arm.body.span, &mut arm_env, ctx, tail_pos)?;

        // Verifica que todos os braços retornam o mesmo tipo.
        if let Some(ref existing) = match_ret_ty {
            if *existing != typed_body.ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{:?}", existing),
                    found: format!("{:?}", typed_body.ty),
                    span: arm.body.span.into(),
                });
            }
        } else {
            match_ret_ty = Some(typed_body.ty.clone());
        }

        typed_arms.push(TypedMatchArm {
            pattern: typed_pattern,
            guard: typed_guard,
            body: Spanned::new(typed_body, arm.body.span),
        });
    }

    let ret_ty = match_ret_ty.ok_or_else(|| MiddleError::TypeMismatch {
        expected: "pelo menos um braço".into(),
        found: "nenhum braço".into(),
        span: (*span).into(),
    })?;

    // Verifica exaustividade.
    patterns::check_exhaustiveness(
        &covered_variants,
        &scrutinee_ty,
        has_otherwise,
        ctx.enum_registry,
        span,
    )?;

    Ok((
        ret_ty.clone(),
        TypedExprKind::Match {
            scrutinee: Box::new(Spanned::new(typed_scrutinee, scrutinee.span)),
            arms: typed_arms,
        },
        Effect::Puro,
    ))
}
