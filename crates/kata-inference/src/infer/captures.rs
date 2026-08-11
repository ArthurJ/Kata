//! Coleta de captures (free variables) de closures.
//!
//! Percorre a TAST pós-inferência e, para cada `Lambda`, coleta as free
//! variables do body contra os bindings locais (params + with bindings +
//! pattern binds). Cada free var vira um `CaptureInfo { name, ty }`.
//!
//! Roda após `infer_module` construir a `TypedModule`, mutando in-place os
//! campos `captures` de cada Lambda.

use std::collections::{HashMap, HashSet};

use kata_core::ty::{Ty, TypeEnv};

use crate::typed::{CaptureInfo, TypedExpr, TypedExprKind, TypedModule};
use crate::typed_pattern::TypedLambdaClause;

use super::free_vars::{collect_free_vars, collect_pattern_binds, collect_pattern_tys};
use super::walk::for_each_subexpr_mut;

/// Ponto de entrada — percorre toda a TAST e popula `captures` de cada Lambda.
///
/// Percorre: pre_entry, entry, functions (clauses), actions (body).
/// Para cada `Lambda`, coleta free vars do body.
pub(crate) fn run(typed_module: &mut TypedModule) {
    let empty_tys = HashMap::new();

    // Separar dispatch_table do resto via split borrow para passar
    // &DispatchTable às chamadas enquanto mutamos exprs dos outros campos.
    let TypedModule {
        pre_entry,
        entry,
        dispatch_table,
        type_env,
        functions,
        actions,
        struct_registry: _,
        snapshots: _,
        refined_decls: _,
        constants,
    } = typed_module;

    let dispatch = &*dispatch_table;

    // Percorre pre_entry
    for expr in pre_entry {
        collect_captures_in_expr(&mut expr.node, type_env, &empty_tys, dispatch);
    }
    // Percorre constants (ConstantBinding pode conter lambdas com captures)
    for expr in constants {
        collect_captures_in_expr(&mut expr.node, type_env, &empty_tys, dispatch);
    }
    // Percorre entry
    collect_captures_in_expr(&mut entry.node, type_env, &empty_tys, dispatch);

    // Percorre funções nomeadas
    for func in functions {
        for clause in &mut func.clauses {
            // Tipos dos bindings locais da cláusula: params + with bindings.
            // Usado para resolver tipos de captures que são bindings locais
            // (ex: `pivo`/`resto` de pattern Cons, `menores`/`maiores` de with).
            let mut local_tys: HashMap<String, Ty> = HashMap::new();
            collect_pattern_tys(&clause.patterns, &mut local_tys);
            for wb in &clause.with_bindings {
                local_tys.insert(wb.name.clone(), wb.value.node.ty.clone());
            }

            // Guard bodies e clause body podem ter captures
            for guard in &mut clause.guards {
                if let Some(cond) = &mut guard.condition {
                    collect_captures_in_expr(&mut cond.node, type_env, &local_tys, dispatch);
                }
                collect_captures_in_expr(&mut guard.body.node, type_env, &local_tys, dispatch);
            }
            // With bindings podem conter lambdas que capturam vars do escopo
            // (ex: `menores := filter (< _ pivo) resto` — o lambda captura `pivo`).
            for wb in &mut clause.with_bindings {
                collect_captures_in_expr(&mut wb.value.node, type_env, &local_tys, dispatch);
            }
            collect_captures_in_expr(&mut clause.body.node, type_env, &local_tys, dispatch);
        }
    }

    // Percorre Actions
    for action in actions {
        for stmt in &mut action.body {
            collect_captures_in_expr(&mut stmt.node, type_env, &empty_tys, dispatch);
        }
    }
}

/// Percorre uma expressão TAST, populando captures de cada Lambda.
///
/// `local_tys` mapeia nomes de bindings do escopo envolvente (params de
/// cláusulas, with bindings, pattern binds) aos seus tipos. Usado como
/// fallback quando `outer_env.lookup(name)` falha — captures que são
/// bindings locais da cláusula (não globais) precisam ter seus tipos
/// resolvidos.
fn collect_captures_in_expr(
    expr: &mut TypedExpr,
    outer_env: &TypeEnv,
    local_tys: &HashMap<String, Ty>,
    dispatch: &kata_core::dispatch::DispatchTable,
) {
    for_each_subexpr_mut(expr, &mut |e| {
        if let TypedExprKind::Lambda {
            clauses,
            captures: lambda_captures,
            ..
        } = &mut e.kind
        {
            // Lambda: boundary de escopo. Assumimos controle total.
            //
            // Primeira passada: coleta free vars (imutável) e popula
            // lambda_captures.
            let mut all_captures: Vec<CaptureInfo> = Vec::new();

            for clause in clauses.iter() {
                collect_clause_free_vars(clause, outer_env, local_tys, dispatch, &mut all_captures);
            }

            all_captures.sort_by(|a, b| a.name.cmp(&b.name));
            *lambda_captures = all_captures;

            // Segunda passada: percorre bodies para Closures aninhadas (mut).
            for clause in clauses.iter_mut() {
                collect_captures_in_lambda_clause(clause, outer_env, local_tys, dispatch);
            }

            return false; // já processamos — não deixa o visitor descer
        }
        true
    });
}

/// Coleta free vars de uma cláusula lambda e adiciona à lista de captures.
fn collect_clause_free_vars(
    clause: &TypedLambdaClause,
    outer_env: &TypeEnv,
    local_tys: &HashMap<String, Ty>,
    dispatch: &kata_core::dispatch::DispatchTable,
    all_captures: &mut Vec<CaptureInfo>,
) {
    let mut local_bindings = HashSet::new();
    collect_pattern_binds(&clause.patterns, &mut local_bindings);
    for wb in &clause.with_bindings {
        local_bindings.insert(wb.name.clone());
    }

    let mut free_vars = HashSet::new();
    if clause.guards.is_empty() {
        collect_free_vars(&clause.body.node, &local_bindings, dispatch, &mut free_vars);
    } else {
        for guard in &clause.guards {
            if let Some(cond) = &guard.condition {
                collect_free_vars(&cond.node, &local_bindings, dispatch, &mut free_vars);
            }
            collect_free_vars(&guard.body.node, &local_bindings, dispatch, &mut free_vars);
        }
    }

    for name in &free_vars {
        if name.starts_with("__") {
            continue;
        }
        if !all_captures.iter().any(|c| &c.name == name) {
            // Resolve o tipo da capture: primeiro no type_env
            // (globais), depois nos bindings locais do escopo.
            if let Some(ty) = outer_env.lookup(name) {
                all_captures.push(CaptureInfo {
                    name: name.clone(),
                    ty: ty.clone(),
                });
            } else if let Some(ty) = local_tys.get(name) {
                all_captures.push(CaptureInfo {
                    name: name.clone(),
                    ty: ty.clone(),
                });
            }
        }
    }
}

/// Percorre o corpo de uma cláusula lambda (segunda passada, mutável)
/// para coletar captures de lambdas aninhados.
fn collect_captures_in_lambda_clause(
    clause: &mut TypedLambdaClause,
    outer_env: &TypeEnv,
    local_tys: &HashMap<String, Ty>,
    dispatch: &kata_core::dispatch::DispatchTable,
) {
    for guard in &mut clause.guards {
        if let Some(cond) = &mut guard.condition {
            collect_captures_in_expr(&mut cond.node, outer_env, local_tys, dispatch);
        }
        collect_captures_in_expr(&mut guard.body.node, outer_env, local_tys, dispatch);
    }
    if clause.guards.is_empty() {
        collect_captures_in_expr(&mut clause.body.node, outer_env, local_tys, dispatch);
    }
}
