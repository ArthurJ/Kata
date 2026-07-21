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

use crate::typed::{CaptureInfo, FusedStage, TypedExpr, TypedExprKind, TypedMatchArm, TypedModule};

use super::free_vars::{collect_free_vars, collect_pattern_binds, collect_pattern_tys};

/// Ponto de entrada — percorre toda a TAST e popula `captures` de cada Lambda.
///
/// Percorre: pre_entry, entry, functions (clauses), actions (body).
/// Para cada `Lambda`, coleta free vars do body.
pub(crate) fn run(typed_module: &mut TypedModule) {
    let empty_tys = HashMap::new();

    // Percorre pre_entry
    for expr in &mut typed_module.pre_entry {
        collect_captures_in_expr(&mut expr.node, &typed_module.type_env, &empty_tys);
    }
    // Percorre entry
    collect_captures_in_expr(
        &mut typed_module.entry.node,
        &typed_module.type_env,
        &empty_tys,
    );

    // Percorre funções nomeadas
    for func in &mut typed_module.functions {
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
                    collect_captures_in_expr(&mut cond.node, &typed_module.type_env, &local_tys);
                }
                collect_captures_in_expr(&mut guard.body.node, &typed_module.type_env, &local_tys);
            }
            // With bindings podem conter lambdas que capturam vars do escopo
            // (ex: `menores := filter (< _ pivo) resto` — o lambda captura `pivo`).
            for wb in &mut clause.with_bindings {
                collect_captures_in_expr(&mut wb.value.node, &typed_module.type_env, &local_tys);
            }
            collect_captures_in_expr(&mut clause.body.node, &typed_module.type_env, &local_tys);
        }
    }

    // Percorre Actions
    for action in &mut typed_module.actions {
        for stmt in &mut action.body {
            collect_captures_in_expr(&mut stmt.node, &typed_module.type_env, &empty_tys);
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
) {
    match &mut expr.kind {
        TypedExprKind::Closure { callee, args, .. } => {
            // Lowera args primeiro (captures aninhadas em args)
            for arg in args {
                collect_captures_in_expr(&mut arg.node, outer_env, local_tys);
            }

            // Recursa no callee — se for Lambda, o braço Lambda abaixo
            // popula lambda.captures diretamente (single source of truth).
            collect_captures_in_expr(&mut callee.node, outer_env, local_tys);
        }

        TypedExprKind::TypeAscription { expr, .. } => {
            collect_captures_in_expr(&mut expr.node, outer_env, local_tys);
        }
        TypedExprKind::Grouping { inner } => {
            collect_captures_in_expr(&mut inner.node, outer_env, local_tys);
        }
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                collect_captures_in_expr(&mut el.node, outer_env, local_tys);
            }
        }
        TypedExprKind::Let { value, .. } | TypedExprKind::Var { value, .. } => {
            collect_captures_in_expr(&mut value.node, outer_env, local_tys);
        }
        TypedExprKind::Reassign { value, .. } => {
            collect_captures_in_expr(&mut value.node, outer_env, local_tys);
        }
        TypedExprKind::Return(inner) => {
            collect_captures_in_expr(&mut inner.node, outer_env, local_tys);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            collect_captures_in_expr(&mut scrutinee.node, outer_env, local_tys);
            for arm in arms {
                collect_captures_in_arm(arm, outer_env, local_tys);
            }
        }
        TypedExprKind::Lambda {
            clauses,
            captures: lambda_captures,
            ..
        } => {
            // Lambda fora de Closure (ex: `let f := lambda x: ...`).
            // Coleta free vars do body contra os bindings locais do lambda.
            let mut all_captures: Vec<CaptureInfo> = Vec::new();

            // Primeira passada: coleta free vars (imutável)
            for clause in clauses.iter() {
                let mut local_bindings = HashSet::new();
                collect_pattern_binds(&clause.patterns, &mut local_bindings);
                for wb in &clause.with_bindings {
                    local_bindings.insert(wb.name.clone());
                }

                let mut free_vars = HashSet::new();
                if clause.guards.is_empty() {
                    collect_free_vars(&clause.body.node, &local_bindings, &mut free_vars);
                } else {
                    for guard in &clause.guards {
                        if let Some(cond) = &guard.condition {
                            collect_free_vars(&cond.node, &local_bindings, &mut free_vars);
                        }
                        collect_free_vars(&guard.body.node, &local_bindings, &mut free_vars);
                    }
                }

                for name in &free_vars {
                    if name.starts_with("__") {
                        continue;
                    }
                    if !all_captures.iter().any(|c| c.name == *name) {
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

            all_captures.sort_by(|a, b| a.name.cmp(&b.name));
            *lambda_captures = all_captures;

            // Segunda passada: percorre bodies para Closures aninhadas (mutável)
            for clause in clauses.iter_mut() {
                for guard in &mut clause.guards {
                    if let Some(cond) = &mut guard.condition {
                        collect_captures_in_expr(&mut cond.node, outer_env, local_tys);
                    }
                    collect_captures_in_expr(&mut guard.body.node, outer_env, local_tys);
                }
                if clause.guards.is_empty() {
                    collect_captures_in_expr(&mut clause.body.node, outer_env, local_tys);
                }
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                collect_captures_in_expr(&mut stmt.node, outer_env, local_tys);
            }
        }
        TypedExprKind::ActionCall { args, .. } => {
            collect_captures_in_expr(&mut args.node, outer_env, local_tys);
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for val in values {
                collect_captures_in_expr(&mut val.node, outer_env, local_tys);
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => {
            collect_captures_in_expr(&mut expr.node, outer_env, local_tys);
        }
        TypedExprKind::IndexAccess { expr, .. } => {
            collect_captures_in_expr(&mut expr.node, outer_env, local_tys);
        }
        // ── Coleções — recursão nos elementos ──
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                collect_captures_in_expr(&mut el.node, outer_env, local_tys);
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            collect_captures_in_expr(&mut start.node, outer_env, local_tys);
            collect_captures_in_expr(&mut step.node, outer_env, local_tys);
            collect_captures_in_expr(&mut end.node, outer_env, local_tys);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            collect_captures_in_expr(&mut iterable.node, outer_env, local_tys);
            for stmt in body {
                collect_captures_in_expr(&mut stmt.node, outer_env, local_tys);
            }
        }
        TypedExprKind::In { item, collection } => {
            collect_captures_in_expr(&mut item.node, outer_env, local_tys);
            collect_captures_in_expr(&mut collection.node, outer_env, local_tys);
        }
        // ── Map/filter/fold — recursão ──
        TypedExprKind::Map {
            callback,
            collection,
            ..
        }
        | TypedExprKind::Filter {
            callback,
            collection,
            ..
        } => {
            collect_captures_in_expr(&mut callback.node, outer_env, local_tys);
            collect_captures_in_expr(&mut collection.node, outer_env, local_tys);
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            collect_captures_in_expr(&mut callback.node, outer_env, local_tys);
            collect_captures_in_expr(&mut initial.node, outer_env, local_tys);
            collect_captures_in_expr(&mut collection.node, outer_env, local_tys);
        }
        // ── FusedStream — recursão ──
        TypedExprKind::FusedStream { stages, source, .. } => {
            collect_captures_in_expr(&mut source.node, outer_env, local_tys);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                collect_captures_in_expr(&mut cb.node, outer_env, local_tys);
            }
        }
        // ── CSP — recursão ──
        TypedExprKind::ChannelSend { channel, value } => {
            collect_captures_in_expr(&mut channel.node, outer_env, local_tys);
            collect_captures_in_expr(&mut value.node, outer_env, local_tys);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            collect_captures_in_expr(&mut channel.node, outer_env, local_tys);
        }
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            for arm in arms {
                collect_captures_in_expr(&mut arm.channel.node, outer_env, local_tys);
                collect_captures_in_expr(&mut arm.body.node, outer_env, local_tys);
            }
            if let Some(tm) = timeout_ms {
                collect_captures_in_expr(&mut tm.node, outer_env, local_tys);
            }
            if let Some(tb) = timeout_body {
                collect_captures_in_expr(&mut tb.node, outer_env, local_tys);
            }
        }
        TypedExprKind::ChannelCreate { .. } => {}
        // ReceiverFactoryCall: o factory é uma sub-expressão (Ident do rxf).
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            collect_captures_in_expr(&mut factory.node, outer_env, local_tys);
        }
        TypedExprKind::Fork { args, .. } => {
            collect_captures_in_expr(&mut args.node, outer_env, local_tys);
        }
        // Folhas sem sub-expressões
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::VariantConstruct { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
    }
}

/// Percorre um TypedMatchArm para coletar captures.
fn collect_captures_in_arm(
    arm: &mut TypedMatchArm,
    outer_env: &TypeEnv,
    local_tys: &HashMap<String, Ty>,
) {
    if let Some(guard) = &mut arm.guard {
        collect_captures_in_expr(&mut guard.node, outer_env, local_tys);
    }
    collect_captures_in_expr(&mut arm.body.node, outer_env, local_tys);
}
