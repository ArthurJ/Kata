//! Inferência de `match` — pattern matching com verificação de exaustividade.
//!
//! Processa scrutinee + arms, casa padrões, infere guards e bodies,
//! verifica uniformidade de tipo entre braços e exaustividade.

use kata_ast::{Expr, MatchArm, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::patterns;
use crate::typed::{TypedExpr, TypedExprKind, TypedMatchArm, TypedPattern};

use super::expr::{InferCtx, infer_expr, infer_expr_hinted};
use super::helpers::InferResult;

/// Unificação limitada para tipos de braços de match.
/// Se um é Ty::Var e o outro é concreto, retorna o concreto.
/// Se ambos são Ty::Var com o mesmo nome, retorna um deles.
/// Caso contrário, retorna None (incompatíveis).
///
/// Recursiva em Generic, Tuple, List, Array, Range, Dict, Set, Function,
/// e tipos de canal (Sender/Receiver/ReceiverFactory): cada type arg é
/// unificado independentemente, permitindo que arms complementares
/// resolvam type params distintos (ex: arm Ok infere T, arm Err infere E).
fn unify_arm_types(a: &Ty, b: &Ty) -> Option<Ty> {
    match (a, b) {
        // Var unifica com qualquer coisa — retorna o outro.
        (Ty::Var(_), Ty::Var(_)) if a == b => Some(a.clone()),
        (Ty::Var(_), _) => Some(b.clone()),
        (_, Ty::Var(_)) => Some(a.clone()),

        // Generic: mesma base, mesma aridade → unifica cada type arg.
        (Ty::Generic(n1, a1), Ty::Generic(n2, a2)) if n1 == n2 && a1.len() == a2.len() => {
            let mut unified = Vec::with_capacity(a1.len());
            for (x, y) in a1.iter().zip(a2.iter()) {
                unified.push(unify_arm_types(x, y)?);
            }
            Some(Ty::Generic(n1.clone(), unified))
        }

        // Tuple: mesma aridade → unifica cada elemento.
        (Ty::Tuple(a1), Ty::Tuple(a2)) if a1.len() == a2.len() => {
            let mut unified = Vec::with_capacity(a1.len());
            for (x, y) in a1.iter().zip(a2.iter()) {
                unified.push(unify_arm_types(x, y)?);
            }
            Some(Ty::Tuple(unified))
        }

        // Function: unifica params e retorno.
        (Ty::Function(p1, r1), Ty::Function(p2, r2)) if p1.len() == p2.len() => {
            let mut unified_params = Vec::with_capacity(p1.len());
            for (x, y) in p1.iter().zip(p2.iter()) {
                unified_params.push(unify_arm_types(x, y)?);
            }
            let unified_ret = unify_arm_types(r1, r2)?;
            Some(Ty::Function(unified_params, Box::new(unified_ret)))
        }

        // Tipos unários: List, Array, Range, Sender, Receiver, ReceiverFactory.
        (Ty::List(a), Ty::List(b)) => Some(Ty::List(Box::new(unify_arm_types(a, b)?))),
        (Ty::Array(a), Ty::Array(b)) => Some(Ty::Array(Box::new(unify_arm_types(a, b)?))),
        (Ty::Range(a), Ty::Range(b)) => Some(Ty::Range(Box::new(unify_arm_types(a, b)?))),
        (Ty::Sender(a), Ty::Sender(b)) => Some(Ty::Sender(Box::new(unify_arm_types(a, b)?))),
        (Ty::Receiver(a), Ty::Receiver(b)) => Some(Ty::Receiver(Box::new(unify_arm_types(a, b)?))),
        (Ty::ReceiverFactory(a), Ty::ReceiverFactory(b)) => {
            Some(Ty::ReceiverFactory(Box::new(unify_arm_types(a, b)?)))
        }

        // Dict: dois type args (K, V).
        (Ty::Dict(k1, v1), Ty::Dict(k2, v2)) => Some(Ty::Dict(
            Box::new(unify_arm_types(k1, k2)?),
            Box::new(unify_arm_types(v1, v2)?),
        )),

        // Set: um type arg.
        (Ty::Set(a), Ty::Set(b)) => Some(Ty::Set(Box::new(unify_arm_types(a, b)?))),

        // Igualdade estrutural para tipos sem aninhamento.
        _ if a == b => Some(a.clone()),
        _ => None,
    }
}

/// Substitui Ty::Var pelos tipos concretos correspondentes em `resolved`.
///
/// Percorre `ty` e `resolved` em paralelo. Onde `ty` tem Ty::Var e `resolved`
/// tem um tipo concreto na mesma posição, substitui. Retorna `resolved`
/// onde `ty == resolved` (não há Var a substituir).
///
/// Isto é a retropropagação: depois que `unify_arm_types` descobre o tipo
/// completo do match (ex: `Result::(Int, Text)`), aplica-o aos arm bodies
/// que foram inferidos com Vars não-resolvidos (ex: `Result::(Int, Var("E"))`).
fn propagate_resolved(ty: &Ty, resolved: &Ty) -> Ty {
    match (ty, resolved) {
        // Var no arm → usa o concreto do resolved.
        (Ty::Var(_), other) => other.clone(),

        // Generic: mesma base, mesma aridade → recursa em cada arg.
        (Ty::Generic(n1, a1), Ty::Generic(n2, a2)) if n1 == n2 && a1.len() == a2.len() => {
            let args = a1
                .iter()
                .zip(a2.iter())
                .map(|(x, y)| propagate_resolved(x, y))
                .collect();
            Ty::Generic(n1.clone(), args)
        }

        // Tuple: mesma aridade → recursa.
        (Ty::Tuple(a1), Ty::Tuple(a2)) if a1.len() == a2.len() => Ty::Tuple(
            a1.iter()
                .zip(a2.iter())
                .map(|(x, y)| propagate_resolved(x, y))
                .collect(),
        ),

        // Function: recursa em params e retorno.
        (Ty::Function(p1, r1), Ty::Function(p2, r2)) if p1.len() == p2.len() => {
            let params = p1
                .iter()
                .zip(p2.iter())
                .map(|(x, y)| propagate_resolved(x, y))
                .collect();
            let ret = Box::new(propagate_resolved(r1, r2));
            Ty::Function(params, ret)
        }

        // Tipos unários.
        (Ty::List(a), Ty::List(b)) => Ty::List(Box::new(propagate_resolved(a, b))),
        (Ty::Array(a), Ty::Array(b)) => Ty::Array(Box::new(propagate_resolved(a, b))),
        (Ty::Range(a), Ty::Range(b)) => Ty::Range(Box::new(propagate_resolved(a, b))),
        (Ty::Sender(a), Ty::Sender(b)) => Ty::Sender(Box::new(propagate_resolved(a, b))),
        (Ty::Receiver(a), Ty::Receiver(b)) => Ty::Receiver(Box::new(propagate_resolved(a, b))),
        (Ty::ReceiverFactory(a), Ty::ReceiverFactory(b)) => {
            Ty::ReceiverFactory(Box::new(propagate_resolved(a, b)))
        }

        // Dict: dois type args.
        (Ty::Dict(k1, v1), Ty::Dict(k2, v2)) => Ty::Dict(
            Box::new(propagate_resolved(k1, k2)),
            Box::new(propagate_resolved(v1, v2)),
        ),

        // Set: um type arg.
        (Ty::Set(a), Ty::Set(b)) => Ty::Set(Box::new(propagate_resolved(a, b))),

        // Sem Var para substituir — mantém o tipo original.
        _ => ty.clone(),
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
            let typed_pat = patterns::check_pattern(
                pat,
                &scrutinee_ty,
                ctx.enum_registry,
                &mut arm_env,
                ctx.interface_registry,
                ctx.struct_registry,
            )?;
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

        // ── Path conditions: coleta facts do braço ──
        // Guard direto: o guard tipado é verdadeiro neste braço.
        // Match sobre Boolean: True → scrutinee é true; False → scrutinee é false.
        // Checkpoint/rollback: facts do braço são adicionados ao RefCell
        // compartilhado e revertidos ao sair (preservando learned_facts).
        let arm_checkpoint = ctx.path_conditions.borrow().checkpoint();
        if let Some(ref guard_spanned) = typed_guard {
            ctx.path_conditions
                .borrow_mut()
                .add_fact(guard_spanned.node.clone());
        }
        if scrutinee_ty == Ty::boolean()
            && let Some(ref pat) = typed_pattern
            && let TypedPattern::Variant {
                ref enum_name,
                ref variant,
                ..
            } = pat.node
            && enum_name == "Boolean"
        {
            match variant.as_str() {
                "True" => {
                    ctx.path_conditions
                        .borrow_mut()
                        .add_fact(typed_scrutinee.clone());
                }
                "False" => {
                    // not(scrutinee) — constrói Closure { not, [scrutinee] }
                    let not_scrut = TypedExpr {
                        span: scrutinee.span,
                        ty: Ty::boolean(),
                        tail_pos: false,
                        escape: typed_scrutinee.escape,
                        kind: TypedExprKind::Closure {
                            callee: Box::new(Spanned::new(
                                TypedExpr {
                                    span: scrutinee.span,
                                    ty: Ty::boolean(),
                                    tail_pos: false,
                                    escape: typed_scrutinee.escape,
                                    kind: TypedExprKind::Ident {
                                        name: "not".to_string(),
                                    },
                                },
                                scrutinee.span,
                            )),
                            args: vec![Spanned::new(typed_scrutinee.clone(), scrutinee.span)],
                            ffi_symbol: None,
                        },
                    };
                    ctx.path_conditions.borrow_mut().add_fact(not_scrut);
                }
                _ => {}
            }
        }

        // ── Nível 2: Post-condições inter-procedurais ──
        // Se o scrutinee é uma chamada de função (Closure, possivelmente
        // dentro de Grouping) e o pattern é um variant de enum, consulta
        // a PostCondTable para extrair a condição que produziu aquele
        // variant e adiciona como path condition.
        if let Some(ref pat) = typed_pattern
            && let TypedPattern::Variant {
                ref enum_name,
                ref variant,
                ref sub_patterns,
                ..
            } = pat.node
        {
            // Desembrulha Grouping para chegar no Closure.
            let scrutinee_inner = match &typed_scrutinee.kind {
                TypedExprKind::Grouping { inner } => &inner.node,
                _ => &typed_scrutinee,
            };
            if let TypedExprKind::Closure { callee, args, .. } = &scrutinee_inner.kind
                && let TypedExprKind::Ident { name: func_name } = &callee.node.kind
                && let Some(post_conds) = ctx.post_conds.get(func_name)
            {
                let arg_types: Vec<Ty> = args.iter().map(|a| a.node.ty.clone()).collect();
                for pc in post_conds {
                    if pc.enum_name == *enum_name
                        && pc.variant == *variant
                        && pc.param_types.len() == arg_types.len()
                        && pc
                            .param_types
                            .iter()
                            .zip(arg_types.iter())
                            .all(|(pt, at)| pt == at)
                    {
                        let substituted = super::post_conditions::substitute_params(
                            &pc.condition,
                            &pc.param_names,
                            args,
                        );
                        ctx.path_conditions.borrow_mut().add_fact(substituted);

                        // Conecta binding do pattern ao payload do variant.
                        // Se o pattern é `Ok n` e o payload (após substituição)
                        // é `7`, adiciona fato `= n 7`. Isso permite ao Z3
                        // provar predicados sobre o binding usando facts sobre
                        // o argumento.
                        if let Some(ref payload) = pc.payload
                            && let Some(subs) = sub_patterns
                            && let Some(first_sub) = subs.first()
                            && let TypedPattern::Ident {
                                name: binding_name, ..
                            } = &first_sub.node
                        {
                            let payload_sub = super::post_conditions::substitute_params(
                                payload,
                                &pc.param_names,
                                args,
                            );
                            // Só adiciona se o payload substituído for simples
                            // (Ident ou IntLit) — evita criar variáveis opacas
                            // que podem interferir com outras provas Z3.
                            if matches!(
                                payload_sub.kind,
                                TypedExprKind::Ident { .. } | TypedExprKind::IntLit { .. }
                            ) {
                                let binding_expr = TypedExpr {
                                    span: first_sub.span,
                                    ty: payload_sub.ty.clone(),
                                    tail_pos: false,
                                    escape: EscapeTarget::Local,
                                    kind: TypedExprKind::Ident {
                                        name: binding_name.clone(),
                                    },
                                };
                                let eq_fact = TypedExpr {
                                    span: first_sub.span,
                                    ty: Ty::boolean(),
                                    tail_pos: false,
                                    escape: EscapeTarget::Local,
                                    kind: TypedExprKind::Closure {
                                        callee: Box::new(Spanned::new(
                                            TypedExpr {
                                                span: first_sub.span,
                                                ty: Ty::boolean(),
                                                tail_pos: false,
                                                escape: EscapeTarget::Local,
                                                kind: TypedExprKind::Ident {
                                                    name: "=".to_string(),
                                                },
                                            },
                                            first_sub.span,
                                        )),
                                        args: vec![
                                            Spanned::new(binding_expr, first_sub.span),
                                            Spanned::new(payload_sub, first_sub.span),
                                        ],
                                        ffi_symbol: None,
                                    },
                                };
                                ctx.path_conditions.borrow_mut().add_fact(eq_fact);
                            }
                        }
                    }
                }
            }
        }

        // InferCtx local com path conditions do braço (RefCell compartilhado).
        let arm_ctx = InferCtx {
            table: ctx.table,
            enum_registry: ctx.enum_registry,
            struct_registry: ctx.struct_registry,
            refined_decls: ctx.refined_decls,
            interface_registry: ctx.interface_registry,
            refines_registry: ctx.refines_registry,
            type_graph: ctx.type_graph,
            ret_ty: ctx.ret_ty,
            in_loop: ctx.in_loop,
            deferred_lambdas: ctx.deferred_lambdas,
            path_conditions: std::rc::Rc::clone(&ctx.path_conditions),
            post_conds: ctx.post_conds,
            inline_fns: ctx.inline_fns,
        };

        // Infere body do braço — propaga hint do contexto (ex: tipo de
        // retorno da função nomeada) para permitir que construções de
        // variant dentro do arm resolvam type params não-inferidos.
        let typed_body = infer_expr_hinted(
            &arm.body.node,
            &arm.body.span,
            &mut arm_env,
            &arm_ctx,
            tail_pos,
            hint,
        )?;
        // Rollback: remove facts de guard do braço, preserva learned_facts.
        ctx.path_conditions.borrow_mut().rollback_to(arm_checkpoint);

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

    // Retropropagação: aplica o tipo resolvido do match aos arm bodies
    // que foram inferidos com Ty::Var não-resolvidos. Isto garante que
    // arms individuais carreguem o tipo completo (ex: Result::(Int, Text))
    // em vez de Result::(Int, Var("E")), mantendo consistência entre o
    // tipo do match e o tipo de cada arm.
    for arm in &mut typed_arms {
        let new_ty = propagate_resolved(&arm.body.node.ty, &ret_ty);
        arm.body.node.ty = new_ty;
    }

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
