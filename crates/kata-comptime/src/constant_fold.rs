//! Substituição de `Ident` de constants nos corpos de functions e actions.
//!
//! Após o fixpoint do comptime pass, `comptime_bindings` contém os valores
//! avaliados de todas as constants do módulo. Este módulo percorre os corpos
//! de functions (clauses: body, guards, with_bindings) e actions (body),
//! substituindo `Ident(name)` pelo literal/snapshot correspondente quando
//! `name` é uma constant do módulo e não está mascarado por um binding local
//! (parâmetro da clause ou with_binding).

use std::collections::{HashMap, HashSet};

use kata_inference::{TypedExpr, TypedExprKind, TypedPattern};

use crate::error::ComptimeError;
use crate::walk::walk_mut;

/// Coleta nomes de bindings de um `TypedPattern` recursivamente.
fn collect_pattern_names(pat: &TypedPattern, names: &mut HashSet<String>) {
    match pat {
        TypedPattern::Ident { name, .. } => {
            names.insert(name.clone());
        }
        TypedPattern::Variant { sub_patterns, .. } => {
            if let Some(subs) = sub_patterns {
                for sp in subs {
                    collect_pattern_names(&sp.node, names);
                }
            }
        }
        TypedPattern::Tuple { elements } => {
            for el in elements {
                collect_pattern_names(&el.node, names);
            }
        }
        TypedPattern::Cons { head, tail } => {
            collect_pattern_names(&head.node, names);
            collect_pattern_names(&tail.node, names);
        }
        TypedPattern::Wildcard | TypedPattern::Nil | TypedPattern::Literal { .. } => {}
    }
}

/// Substitui `Ident(name)` por `comptime_bindings[name]` num `TypedExpr`,
/// exceto quando `name` está em `local_names` (parâmetros/with_bindings/let).
fn replace_constant_refs_in_expr(
    expr: &mut TypedExpr,
    comptime_bindings: &HashMap<String, TypedExpr>,
    local_names: &HashSet<String>,
) -> Result<(), ComptimeError> {
    // Primeiro, checar o próprio nó (walk_mut só visita filhos).
    if let TypedExprKind::Ident { name } = &expr.kind
        && !local_names.contains(name)
        && let Some(replacement) = comptime_bindings.get(name)
    {
        expr.kind = replacement.kind.clone();
        expr.ty = replacement.ty.clone();
    }
    // Depois, recursão nos filhos.
    walk_mut(expr, &mut |child| {
        replace_constant_refs_in_expr(child, comptime_bindings, local_names)
    })?;
    Ok(())
}

/// Percorre todas as functions do TypedModule, substituindo refs a constants
/// nos corpos (body, guards, with_bindings) de cada clause.
pub(crate) fn fold_constant_refs_in_functions(
    functions: &mut [kata_inference::TypedFunction],
    comptime_bindings: &HashMap<String, TypedExpr>,
) -> Result<(), ComptimeError> {
    for func in functions.iter_mut() {
        for clause in &mut func.clauses {
            // Coletar nomes de parâmetros da clause.
            let mut local_names = HashSet::new();
            for pat in &clause.patterns {
                collect_pattern_names(&pat.node, &mut local_names);
            }
            // with_bindings também introduzem nomes locais.
            for wb in &clause.with_bindings {
                local_names.insert(wb.name.clone());
            }

            // Substituir no body (se não há guards).
            if clause.guards.is_empty() {
                replace_constant_refs_in_expr(
                    &mut clause.body.node,
                    comptime_bindings,
                    &local_names,
                )?;
            } else {
                // Substituir em cada guard (condition + body).
                for guard in &mut clause.guards {
                    if let Some(cond) = &mut guard.condition {
                        replace_constant_refs_in_expr(
                            &mut cond.node,
                            comptime_bindings,
                            &local_names,
                        )?;
                    }
                    replace_constant_refs_in_expr(
                        &mut guard.body.node,
                        comptime_bindings,
                        &local_names,
                    )?;
                }
            }

            // Substituir nos values dos with_bindings.
            // with_binding value pode referenciar constants, mas o name
            // do with_binding ainda não está em scope dentro do seu próprio
            // value — então usamos local_names sem o name do próprio wb.
            // Na prática, with_binding value raramente referencia constants,
            // mas é correto percorrer.
            for wb in &mut clause.with_bindings {
                // O value do with_binding é avaliado antes do name entrar
                // em scope, então o name não mascara dentro do value.
                let mut value_locals = local_names.clone();
                value_locals.remove(&wb.name);
                replace_constant_refs_in_expr(
                    &mut wb.value.node,
                    comptime_bindings,
                    &value_locals,
                )?;
            }
        }
    }
    Ok(())
}

/// Percorre todas as actions do TypedModule, substituindo refs a constants
/// nos statements do body. Actions não têm parâmetros como patterns de
/// function — seus parâmetros vêm da assinatura da action, que é tratada
/// pelo codegen separadamente. Os names de parâmetros da action não estão
/// em `comptime_bindings` (são bindings de runtime, não constants), então
/// a substituição é segura.
pub(crate) fn fold_constant_refs_in_actions(
    actions: &mut [kata_inference::TypedAction],
    comptime_bindings: &HashMap<String, TypedExpr>,
) -> Result<(), ComptimeError> {
    for action in actions.iter_mut() {
        // Coletar nomes de parâmetros da action.
        let local_names: HashSet<String> = action
            .param_names
            .iter()
            .filter_map(|p| p.clone())
            .collect();

        for stmt in &mut action.body {
            replace_constant_refs_in_expr(&mut stmt.node, comptime_bindings, &local_names)?;
        }
    }
    Ok(())
}
