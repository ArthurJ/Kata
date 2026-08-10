//! Inferência de lambda anônimo.
//!
//! Para lambda anônimo, os tipos dos parâmetros são inferidos via:
//! 1. Hint top-down (DoD 29) — ascription fornece tipos dos params
//! 2. Partial dispatch (DoD 27) — body é Apply, args revelam tipos
//! 3. Erro LambdaInferenceFail (DoD 30) se nenhum mecanismo resolve

use kata_ast::{Expr, GuardClause, Pattern, Span, Spanned, WithBinding};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind, TypedLambdaClause};

use super::apply_lambda::infer_lambda_body;
use super::expr::InferCtx;
use super::helpers::{peel_grouping_expr, InferResult, check_patterns, process_with_bindings};
use super::partial_dispatch::{
    PartialDispatchOutcome, PartialDispatchReason, try_partial_dispatch,
};

/// Infere um lambda anônimo ou cláusula lambda.
///
/// Para lambda anônimo: 1 cláusula, sem nome de função.
/// Para função nomeada (cláusulas de Sig): múltiplas cláusulas, com nome.
///
/// Lambda anônimo não tem assinatura — os tipos dos parâmetros
/// são inferidos a partir do primeiro uso. Para funções nomeadas, a
/// assinatura fornece os tipos. Aqui tratamos apenas lambda anônimo
/// (sem assinatura); funções nomeadas são tratadas em `infer_named_function`
/// no `mod.rs`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_lambda(
    patterns: &[Spanned<Pattern>],
    body: &Spanned<Expr>,
    guards: &[GuardClause],
    with_bindings: &[WithBinding],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    hint: Option<&Ty>,
) -> InferResult<(Ty, TypedExprKind)> {
    // Para lambda anônimo, os tipos dos parâmetros são InferVar — não temos
    // inferência de tipos real ainda. O lambda anônimo só funciona
    // quando o tipo é determinado pelo contexto (ex: `let f := lambda x: + x 1`
    // infere x:Int porque + exige Int). Mas sem inferência bidirecional, isso
    // não é possível. usa uma abordagem simples: InferVar e unificação
    // não estão implementados — o lambda ganha tipos InferVar e o primeiro
    // uso determina o tipo.
    //
    // Para , implementamos a estrutura do TypedLambdaClause mas a
    // inferência de tipos do lambda é limitada: cada padrão Ident ganha
    // InferVar e o body é inferido no escopo. O tipo de retorno é o tipo
    // do body. O tipo do lambda é Function(param_types, ret_ty).

    // Cria escopo filho para os bindings do lambda.
    let mut lambda_env = env.push_scope();

    // Tenta inferir tipos dos parâmetros via partial dispatch (DoD 27).
    // Se o body é um Apply com callee Ident, e alguns args são parâmetros do
    // lambda (Ident com nome = nome do pattern), tenta resolve_partial com
    // None nessas posições e tipos concretos nas demais.
    let partial_outcome =
        try_partial_dispatch(patterns, body, env, ctx.table, ctx.interface_registry);

    // DoD 29: Hint top-down via ascription em lambda.
    // O hint tem PRIORIDADE sobre partial dispatch — a anotação explícita
    // do programador (`(lambda ...)::(Int -> Int)`) vence a inferência
    // bottom-up. Se o body não type-checka com os tipos hinted, isso é
    // um erro legítimo de tipo.
    //
    // O hint também desambigua overloads cross-type: `map (+ 10 _) [1 2 3]`
    // fornece hint `Function([Int], ?)` que seleciona `Int Int → Int` entre
    // as múltiplas overloads de `+`.
    let hint_has_params = matches!(hint, Some(Ty::Function(hp, _)) if !hp.is_empty());

    // Fase 2 (PRD OverloadSet): se partial dispatch é ambíguo (múltiplas
    // overloads casam) E não há hint útil, constrói Ty::OverloadSet com as
    // projeções e defere o lambda. O tipo do lambda é OverloadSet, não
    // Function([InferVar], ...). O call site seleciona a overload correta
    // pelo tipo concreto dos args.
    if !hint_has_params {
        if let PartialDispatchOutcome::Ambiguous(projections) = &partial_outcome {
            // Extrai o nome do callee do body (ex: "+" em `+ __hole_0 2`).
            let callee_name = extract_callee_name(body).unwrap_or_else(|| "__unknown".to_string());

            let overload_set_ty = Ty::OverloadSet {
                name: callee_name.clone(),
                overloads: projections.clone(),
            };

            // Constrói skeleton do lambda com InferVar nos params.
            let n_params = patterns.len();
            let param_types: Vec<Ty> = (0..n_params).map(|i| Ty::InferVar(i as u32)).collect();
            let ret_ty = Ty::InferVar(n_params as u32);

            let typed_patterns = patterns
                .iter()
                .enumerate()
                .map(|(i, pat)| {
                    let name = if let Pattern::Ident(n) = &pat.node {
                        n.clone()
                    } else {
                        format!("__param_{i}")
                    };
                    Spanned::new(
                        crate::typed::TypedPattern::Ident {
                            name,
                            ty: Ty::InferVar(i as u32),
                        },
                        patterns[i].span,
                    )
                })
                .collect();

            let skeleton_kind = TypedExprKind::Lambda {
                func_name: None,
                param_types: param_types.clone(),
                ret_ty: ret_ty.clone(),
                clauses: vec![TypedLambdaClause {
                    patterns: typed_patterns,
                    body: Spanned::new(
                        TypedExpr {
                            span: body.span,
                            ty: ret_ty.clone(),
                            tail_pos: true,
                            escape: EscapeTarget::Local,
                            kind: TypedExprKind::Unit,
                        },
                        body.span,
                    ),
                    guards: Vec::new(),
                    with_bindings: Vec::new(),
                }],
                captures: Vec::new(),
            };

            return Ok((overload_set_ty, skeleton_kind));
        }
    }

    // DoD 29: Hint top-down via ascription em lambda.
    // O hint tem PRIORIDADE sobre partial dispatch — a anotação explícita
    // do programador (`(lambda ...)::(Int -> Int)`) vence a inferência
    // bottom-up. Se o body não type-checka com os tipos hinted, isso é
    // um erro legítimo de tipo.
    let (param_type_hints, failure_ctx) = if let Some(Ty::Function(hint_params, _)) = hint {
        if !hint_params.is_empty() {
            (hint_params.clone(), None)
        } else {
            extract_partial(partial_outcome)
        }
    } else {
        extract_partial(partial_outcome)
    };

    // DoD 30: LambdaInferenceFail — se nenhum mecanismo resolveu os tipos
    // dos parâmetros (partial dispatch vazio, sem hint), produzir erro
    // distinto em vez de criar InferVar e deixar o dispatch falhar com
    // NoOverload opaco.
    // Exceção: se o pattern tem type annotation explícita (TypedIdent),
    // o tipo anotado conta como mecanismo de inferência.
    let has_typed_idents = patterns
        .iter()
        .any(|p| matches!(p.node, Pattern::TypedIdent { .. }));
    if !has_typed_idents && param_type_hints.len() < patterns.len() {
        return Err(MiddleError::LambdaInferenceFail {
            span: (*span).into(),
            detail: failure_ctx.map(format_failure_detail),
        });
    }

    // Processa padrões — usa hint do partial dispatch se disponível, senão
    // extrai tipo de TypedIdent annotations, senão InferVar.
    let param_types: Vec<Ty> = patterns
        .iter()
        .enumerate()
        .map(|(i, pat)| {
            if let Some(hint_ty) = param_type_hints.get(i) {
                return hint_ty.clone();
            }
            // Se o pattern tem type annotation (TypedIdent), usar o tipo anotado.
            if let Pattern::TypedIdent { ty, .. } = &pat.node {
                return kata_resolution::resolve_type_expr(
                    &ty.node,
                    &lambda_env,
                    ctx.interface_registry,
                );
            }
            Ty::InferVar(i as u32)
        })
        .collect();
    let typed_patterns = check_patterns(
        patterns,
        &param_types,
        ctx.enum_registry,
        &mut lambda_env,
        ctx.interface_registry,
    )?;

    // Processa with bindings (açúcar → let chain no escopo do lambda).
    // with bindings são pré-avaliados antes dos guards.
    let typed_with_bindings = process_with_bindings(with_bindings, &mut lambda_env, ctx)?;

    // Infere o corpo do lambda — delega para infer_lambda_body (fatorado).
    // O body de um lambda é sempre tail_pos=true dentro do lambda, pois
    // representa o valor de retorno do lambda. O tail_pos do contexto onde
    // o lambda aparece é irrelevante para a posição de cauda do corpo.
    // Extrai o tipo de retorno do hint se for Ty::Function.
    let ret_hint = hint.map(|h| match h {
        Ty::Function(_, ret) => ret.as_ref(),
        _ => h,
    });
    let (ret_ty, typed_body, typed_guards) =
        infer_lambda_body(body, guards, &mut lambda_env, ctx, ret_hint)?;

    let lambda_ty = Ty::Function(param_types.clone(), Box::new(ret_ty.clone()));

    let clause = TypedLambdaClause {
        patterns: typed_patterns,
        body: Spanned::new(typed_body, body.span),
        guards: typed_guards,
        with_bindings: typed_with_bindings,
    };

    Ok((
        lambda_ty,
        TypedExprKind::Lambda {
            func_name: None, // lambda anônimo — atribui nome para Sig
            param_types,
            ret_ty,
            clauses: vec![clause],
            captures: Vec::new(),
        },
    ))
}

/// Extrai `(Vec<Ty>, Option<PartialDispatchFailure>)` do outcome.
///
/// Se `Inferred`, retorna os tipos e None.
/// Se `Ambiguous`, retorna Vec vazio e None — o caller (`infer_lambda`)
/// verifica o outcome diretamente antes de chamar `extract_partial` para
/// construir o `Ty::OverloadSet`.
/// Se `Failed`, retorna Vec vazio e Some(failure) — o caller vai produzir
/// LambdaInferenceFail com o contexto.
/// Se `NotApplicable`, retorna Vec vazio e None — nenhum contexto disponível.
fn extract_partial(
    outcome: PartialDispatchOutcome,
) -> (
    Vec<Ty>,
    Option<super::partial_dispatch::PartialDispatchFailure>,
) {
    match outcome {
        PartialDispatchOutcome::Inferred(tys) => (tys, None),
        PartialDispatchOutcome::Ambiguous(_) => (Vec::new(), None),
        PartialDispatchOutcome::Failed(f) => (Vec::new(), Some(f)),
        PartialDispatchOutcome::NotApplicable => (Vec::new(), None),
    }
}

/// Formata o contexto de falha do partial dispatch como string de diagnóstico.
fn format_failure_detail(f: super::partial_dispatch::PartialDispatchFailure) -> String {
    let args_str = f
        .arg_types
        .iter()
        .map(|a| a.clone().unwrap_or_else(|| "?".into()))
        .collect::<Vec<_>>()
        .join(", ");

    match f.reason {
        PartialDispatchReason::NoOverload { overloads } => {
            if overloads.is_empty() {
                format!(
                    "partial dispatch tentou `{}` com args [{}] mas não resolveu todos os parâmetros",
                    f.callee, args_str
                )
            } else {
                format!(
                    "partial dispatch tentou `{}` com args [{}] — nenhuma overload casa. Overloads: {}",
                    f.callee,
                    args_str,
                    overloads.join(", ")
                )
            }
        }
        PartialDispatchReason::Ambiguous => {
            format!(
                "partial dispatch tentou `{}` com args [{}] — múltiplas overloads casam (ambíguo)",
                f.callee, args_str
            )
        }
    }
}

/// Extrai o nome do callee de um body que é `Apply(Ident(name), ...)`.
/// Usado para nomear o `Ty::OverloadSet` quando partial dispatch é ambíguo.
fn extract_callee_name(body: &Spanned<Expr>) -> Option<String> {
    let body_core = peel_grouping_expr(&body.node);
    if let Expr::Apply { callee, .. } = body_core {
        let callee_core = peel_grouping_expr(&callee.node);
        if let Expr::Ident { name } = callee_core {
            return Some(name.clone());
        }
    }
    None
}
