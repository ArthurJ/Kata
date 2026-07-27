//! Inferência de `match` — pattern matching com verificação de exaustividade.
//!
//! Processa scrutinee + arms, casa padrões, infere guards e bodies,
//! verifica uniformidade de tipo entre braços e exaustividade.

use kata_ast::{Expr, MatchArm, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::patterns;
use crate::typed::{TypedExprKind, TypedMatchArm, TypedPattern};

use super::expr::{InferCtx, infer_expr, infer_expr_hinted};
use super::helpers::InferResult;

/// Unificação limitada para tipos de braços de match.
/// Se um é Ty::Var e o outro é concreto, retorna o concreto.
/// Se ambos são Ty::Var com o mesmo nome, retorna um deles.
/// Caso contrário, retorna None (incompatíveis).
fn unify_arm_types(a: &Ty, b: &Ty) -> Option<Ty> {
    match (a, b) {
        (Ty::Var(_), Ty::Var(_)) if a == b => Some(a.clone()),
        (Ty::Var(_), _) => Some(b.clone()),
        (_, Ty::Var(_)) => Some(a.clone()),
        _ if a == b => Some(a.clone()),
        _ => None,
    }
}

/// Infere um `match` — pattern matching com verificação de exaustividade.
///
/// `hint` é o tipo esperado do match no contexto (ex: tipo de retorno da
/// função nomeada). É propagado para a inferência do body de cada arm,
/// permitindo que construções de variant dentro dos arms recebam o tipo
/// esperado e resolvam type params não-inferidos pelo payload.
pub(crate) fn infer_match(
    scrutinee: &Spanned<Expr>,
    arms: &[MatchArm],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    hint: Option<&Ty>,
) -> InferResult<(Ty, TypedExprKind)> {
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
            // Cons e Nil são variantes virtuais de List — coletar para
            // exaustividade (Cons + Nil = exaustivo, sem otherwise).
            if let TypedPattern::Cons { .. } = &typed_pat.node {
                covered_variants.push("Cons".to_string());
            }
            if let TypedPattern::Nil = &typed_pat.node {
                covered_variants.push("Nil".to_string());
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
                    found: format!("{}", guard_typed.ty),
                    span: guard_expr.span.into(),
                });
            }
            Some(Spanned::new(guard_typed, guard_expr.span))
        } else {
            None
        };

        // Infere body do braço — propaga hint do contexto (ex: tipo de
        // retorno da função nomeada) para permitir que construções de
        // variant dentro do arm resolvam type params não-inferidos.
        let typed_body = infer_expr_hinted(
            &arm.body.node,
            &arm.body.span,
            &mut arm_env,
            ctx,
            tail_pos,
            hint,
        )?;

        // Verifica que todos os braços retornam o mesmo tipo.
        // Unificação limitada — Ty::Var unifica com qualquer tipo concreto.
        // Return é bottom — unifica com qualquer tipo (braço que aborta).
        if let Some(ref existing) = match_ret_ty {
            if *existing != typed_body.ty {
                // Return unifica com qualquer coisa — o braço aborta.
                if matches!(typed_body.kind, TypedExprKind::Return(_)) {
                    // Mantém o tipo existente (o braço Return não contribui).
                } else {
                    // Tenta unificar: se um é Var e o outro é concreto, usa o concreto.
                    let unified = unify_arm_types(existing, &typed_body.ty);
                    match unified {
                        Some(ty) => match_ret_ty = Some(ty),
                        None => {
                            return Err(MiddleError::TypeMismatch {
                                expected: format!("{}", existing),
                                found: format!("{}", typed_body.ty),
                                span: arm.body.span.into(),
                            });
                        }
                    }
                }
            }
        } else if !matches!(typed_body.kind, TypedExprKind::Return(_)) {
            // Primeiro braço não-Return define o tipo do match.
            match_ret_ty = Some(typed_body.ty.clone());
        }
        // Se match_ret_ty é None e o braço é Return, não define — espera o próximo braço.

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
    ))
}
