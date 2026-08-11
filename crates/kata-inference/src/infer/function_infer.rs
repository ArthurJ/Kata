//! Inferência de funções nomeadas com corpo Kata (múltiplas cláusulas).
//!
//! Extraído de `mod.rs` — espelha o padrão de `action_infer.rs`.
//! Cada cláusula é inferida com os tipos da assinatura, padrões casados
//! contra tipos dos parâmetros, e body inferido em escopo filho.

use kata_ast::{GuardClause, Spanned, WithBinding};
use kata_core::ty::{TypeEnv, Ty};
use kata_diagnostics::MiddleError;
use kata_resolution::FunctionDef;

use crate::typed::{TypedFunction, TypedLambdaClause};

use super::apply_lambda::infer_lambda_body;
use super::expr::{InferCtx, infer_expr_hinted};
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
        )?;

        // Processa with bindings (açúcar → let chain).
        let typed_with_bindings =
            process_with_bindings(&desugared_with_bindings, &mut clause_env, ctx)?;

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
            if typed_body.ty != *ret_ty {
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
            if guard_ret != *ret_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{}", ret_ty),
                    found: format!("{}", guard_ret),
                    span: clause_inner.body.span.into(),
                });
            }
            (typed_body, guards)
        };

        typed_clauses.push(TypedLambdaClause {
            patterns: typed_patterns,
            body: Spanned::new(typed_body, clause_inner.body.span),
            guards: typed_guards,
            with_bindings: typed_with_bindings,
        });
    }

    // DoD 12: Verifica sobreposição de cláusulas (RedundantClause).
    // Uma cláusula é redundante se uma cláusula anterior já cobre todos
    // os valores que ela casaria, e a cláusula posterior não tem guards
    // (sem condição adicional que a diferenciaria).
    crate::redundancy::check_redundant_clauses(&typed_clauses)?;

    // Sintetiza log spec se a função tem @log.
    let log = if let Some(log_spec) = &func_def.log {
        // Extrai nomes dos params dos patterns da primeira cláusula.
        let param_names: Vec<String> = typed_clauses
            .first()
            .map(|c| {
                c.patterns
                    .iter()
                    .filter_map(|p| match &p.node {
                        crate::typed_pattern::TypedPattern::Ident { name, .. } => {
                            Some(name.clone())
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Cria escopo com bindings dos params para inferir o template.
        let mut log_env = TypeEnv::with_parent(module_type_env.clone());
        for (i, ty) in param_types.iter().enumerate() {
            log_env.define(&format!("__param_{i}"), ty.clone(), "__local__");
        }
        // Se há nomes de params, define-os também (associados por posição).
        for (i, name) in param_names.iter().enumerate() {
            if let Some(ty) = param_types.get(i) {
                log_env.define(name, ty.clone(), "__local__");
            }
        }
        Some(super::log_synthesis::synthesize_log_spec(
            log_spec,
            &param_names,
            &mut log_env,
            ctx,
        )?)
    } else {
        None
    };

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
        log,
        cache_spec: func_def
            .cache_strategy
            .as_ref()
            .map(|s| crate::typed_module::CacheSpec {
                strategy: s.clone(),
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
        Ty::Struct(name) => name,
        Ty::Sum(name) => name,
        _ => "",
    }
}