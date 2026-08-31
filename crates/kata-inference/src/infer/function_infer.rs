//! Inferência de funções nomeadas com corpo Kata (múltiplas cláusulas).
//!
//! Extraído de `mod.rs` — espelha o padrão de `action_infer.rs`.
//! Cada cláusula é inferida com os tipos da assinatura, padrões casados
//! contra tipos dos parâmetros, e body inferido em escopo filho.

use kata_ast::{GuardClause, Spanned, WithBinding};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::FunctionDef;

use crate::typed::{TypedExpr, TypedFunction, TypedLambdaClause};
use crate::typed_pattern::TypedPattern;

use super::apply_lambda::infer_lambda_body;
use super::expr::{InferCtx, fits_return, infer_expr_hinted};
use super::helpers::{InferResult, check_patterns, process_with_bindings};

/// Infere uma função nomeada com corpo Kata (múltiplas cláusulas).
///
/// Cada cláusula é inferida com os tipos da assinatura (param_types/ret_ty
/// do Sig). Os padrões são casados contra os tipos dos parâmetros. O corpo
/// de cada cláusula é inferido em escopo filho com os bindings dos padrões.
pub(crate) fn infer_named_function(
    func_def: &FunctionDef,
    ctx: &InferCtx,
    module_type_env: &TypeEnv,
) -> InferResult<TypedFunction> {
    let param_types = &func_def.param_types;
    let ret_ty = &func_def.return_type;

    let mut typed_clauses: Vec<TypedLambdaClause> = Vec::new();

    for clause in &func_def.clauses {
        let clause_inner = &clause.node;

        // Cria escopo filho para a cláusula, com acesso aos tipos do módulo
        // (prelude + user). Sem parent, VariantQual como `Boolean::True` falha
        // com UnboundName — o typeck precisa resolver o nome do enum no TypeEnv.
        let mut clause_env = TypeEnv::with_parent(module_type_env.clone());

        // Desugar: elimina Pipe e Hole da cláusula antes do typeck.
        // O parser produz Expr::Hole para `_` em expressões como `< _ pivo`;
        // o desugar converte isso em Lambda. Sem isso, o typeck vê Hole cru
        // e falha com "Hole deve ter sido desugared".
        let desugared_body = crate::desugar::desugar(&clause_inner.body);
        let desugared_guards: Vec<GuardClause> = clause_inner
            .guards
            .iter()
            .map(|g| GuardClause {
                condition: g.condition.as_ref().map(crate::desugar::desugar),
                body: crate::desugar::desugar(&g.body),
            })
            .collect();
        let desugared_with_bindings: Vec<WithBinding> = clause_inner
            .with_bindings
            .iter()
            .map(|w| WithBinding {
                name: w.name.clone(),
                value: crate::desugar::desugar(&w.value),
            })
            .collect();

        // Casa padrões contra tipos dos parâmetros.
        let typed_patterns = check_patterns(
            &clause_inner.patterns,
            param_types,
            ctx.enum_registry,
            &mut clause_env,
            ctx.interface_registry,
            ctx.struct_registry,
            ctx.refined_decls,
        )?;

        // Define `__param_{i}` no escopo da cláusula para que diretivas
        // customizadas possam sintetizar `_args := (__param_0, __param_1, ...)`.
        // Funções puras não nomeiam params na assinatura — `__param_{i}` é o
        // identificador posicional usado pelo desugar de diretivas.
        for (i, ty) in param_types.iter().enumerate() {
            clause_env.define(&format!("__param_{i}"), ty.clone(), "__local__");
        }

        // Processa with bindings (açúcar → let chain).
        let typed_with_bindings =
            process_with_bindings(&desugared_with_bindings, &mut clause_env, ctx)?;

        // Infere synthetic_pre (diretivas Enter): cada expr é inferida no
        // escopo da cláusula. Não são tail_pos nem têm hint de retorno.
        let mut typed_synthetic_pre: Vec<Spanned<TypedExpr>> = Vec::new();
        for e in &clause_inner.synthetic_pre {
            let desugared = crate::desugar::desugar(e);
            let typed = infer_expr_hinted(
                &desugared.node,
                &desugared.span,
                &mut clause_env,
                ctx,
                false, // synthetic_pre não é tail_pos
                None,  // sem hint de retorno
            )?;
            typed_synthetic_pre.push(Spanned::new(typed, e.span));
        }

        // Infere body (com ou sem guards).
        let (typed_body, typed_guards) = if desugared_guards.is_empty() {
            let typed_body = infer_expr_hinted(
                &desugared_body.node,
                &desugared_body.span,
                &mut clause_env,
                ctx,
                true,         // tail_pos = true em body de função
                Some(ret_ty), // hint = tipo de retorno da assinatura
            )?;
            // Verifica que o body retorna o tipo esperado.
            if !fits_return(&typed_body.ty, ret_ty) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{}", ret_ty),
                    found: format!("{}", typed_body.ty),
                    span: clause_inner.body.span.into(),
                });
            }
            (typed_body, Vec::new())
        } else {
            let (guard_ret, typed_body, guards) = infer_lambda_body(
                &desugared_body,
                &desugared_guards,
                &mut clause_env,
                ctx,
                Some(ret_ty),
            )?;
            if !fits_return(&guard_ret, ret_ty) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{}", ret_ty),
                    found: format!("{}", guard_ret),
                    span: clause_inner.body.span.into(),
                });
            }
            (typed_body, guards)
        };

        // Infere synthetic_post (diretivas Exit): cada expr é inferida no
        // escopo da cláusula com `_return` bindado ao tipo de retorno.
        // synthetic_post não é tail_pos nem tem hint de retorno.
        if !clause_inner.synthetic_post.is_empty() {
            clause_env.define("_return", ret_ty.clone(), "__local__");
        }
        let mut typed_synthetic_post: Vec<Spanned<TypedExpr>> = Vec::new();
        for e in &clause_inner.synthetic_post {
            let desugared = crate::desugar::desugar(e);
            let typed = infer_expr_hinted(
                &desugared.node,
                &desugared.span,
                &mut clause_env,
                ctx,
                false, // synthetic_post não é tail_pos
                None,  // sem hint de retorno
            )?;
            typed_synthetic_post.push(Spanned::new(typed, e.span));
        }

        typed_clauses.push(TypedLambdaClause {
            patterns: typed_patterns,
            body: Spanned::new(typed_body, clause_inner.body.span),
            synthetic_pre: typed_synthetic_pre,
            synthetic_post: typed_synthetic_post,
            guards: typed_guards,
            with_bindings: typed_with_bindings,
        });
    }

    // DoD 12: Verifica sobreposição de cláusulas (RedundantClause).
    // Roda ANTES da verificação de exaustividade de guards para poder
    // ver cláusulas com guards não-tautológicos (sem otherwise).
    crate::redundancy::check_redundant_clauses(
        &typed_clauses,
        param_types,
        ctx.enum_registry,
        Some(ctx.inline_fns),
    )?;

    // Verifica exaustividade de cláusulas lambda via motor Maranget + Z3.
    // Múltiplas cláusulas lambda são semanticamente equivalentes a um `match`
    // sobre os parâmetros. O motor Maranget desce payloads de variantes
    // (Some True -> Some consome True como sub-pattern), e quando a estrutura
    // cobre mas há guards, Z3 prova que a disjunção dos guards de todas as
    // cláusulas que casam cada folha é tautologia (Fase 3).
    check_clause_exhaustiveness(&typed_clauses, param_types, ctx, &func_def.clauses)?;

    // Validação de @cache: suporta qualquer tipo — a serialização da cache
    // key é feita via type descriptor (function_def.rs::build_type_descriptor)
    // que cobre Int, Float, Text, List, Struct, Tuple. Sum/Generic serializa
    // tag + payload cru (limitação documentada — payload complexo não
    // serializado recursivamente sem enum_registry no codegen).

    Ok(TypedFunction {
        name: func_def.name.clone(),
        param_types: param_types.clone(),
        ret_ty: ret_ty.clone(),
        clauses: typed_clauses,
        cache_spec: func_def.cache_strategy.as_ref().map(|s| {
            use crate::typed_module::CacheStrategy;
            crate::typed_module::CacheSpec {
                strategy: match s.as_str() {
                    "FIFO" => CacheStrategy::FIFO,
                    "MRU" => CacheStrategy::MRU,
                    "LFU" => CacheStrategy::LFU,
                    _ => CacheStrategy::LRU, // LRU default — resolution valida
                },
                capacity: func_def.cache_capacity.unwrap_or(256),
            }
        }),
        timer_spec: func_def.timer.clone(),
    })
}

/// Extrai o nome de um `Ty` para validação de `refines`.
/// `Ty::Prim(PrimTy::Int)` → `"Int"`, `Ty::Struct(name)` → `name`,
/// `Ty::Sum(name)` → `name`. Outros → `""` (não valida).
pub(crate) fn ty_name(ty: &Ty) -> &str {
    match ty {
        Ty::Prim(kata_core::ty::PrimTy::Int) => "Int",
        Ty::Prim(kata_core::ty::PrimTy::Float) => "Float",
        Ty::Prim(kata_core::ty::PrimTy::Rational) => "Rational",
        Ty::Prim(kata_core::ty::PrimTy::Text) => "Text",
        Ty::Struct(key) => key.name(),
        Ty::Sum(name) => name,
        _ => "",
    }
}

/// Verifica exaustividade de cláusulas lambda via motor Maranget + Z3.
///
/// Múltiplas cláusulas lambda são semanticamente equivalentes a um `match`
/// sobre os parâmetros. O motor Maranget desce payloads de variantes
/// (Some True -> Some consome True como sub-pattern). Quando a estrutura
/// cobre mas há guards, Z3 prova que a disjunção dos guards de todas as
/// cláusulas que casam cada folha é tautologia (Fase 3).
fn check_clause_exhaustiveness(
    typed_clauses: &[TypedLambdaClause],
    param_types: &[Ty],
    ctx: &InferCtx,
    ast_clauses: &[Spanned<kata_ast::LambdaClause>],
) -> InferResult<()> {
    if typed_clauses.is_empty() || param_types.is_empty() {
        return Ok(());
    }

    // Verifica se há otherwise (pattern Ident/Wildcard em qualquer cláusula).
    let mut has_otherwise = false;
    for clause in typed_clauses {
        if clause
            .patterns
            .iter()
            .any(|p| matches!(p.node, TypedPattern::Ident { .. } | TypedPattern::Wildcard))
        {
            has_otherwise = true;
        }
    }

    // Span da primeira cláusula para a mensagem de erro.
    let span = ast_clauses
        .first()
        .map(|c| c.span)
        .unwrap_or(kata_ast::Span::zero());

    // Constrói CapsIndex a partir de InferCtx para o motor Maranget.
    let caps_index = kata_core::CapsIndex::new(ctx.type_graph, ctx.struct_registry);

    crate::maranget::check_exhaustiveness_with_guards(
        typed_clauses,
        param_types,
        has_otherwise,
        &span,
        ctx.enum_registry,
        Some(ctx.struct_registry),
        Some(ctx.refined_decls),
        Some(&caps_index),
        Some(ctx.inline_fns),
    )?;

    Ok(())
}
