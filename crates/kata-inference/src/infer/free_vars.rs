//! Coleta de free variables e pattern binds — extraído de captures.rs.
//!
//! `collect_free_vars` percorre a TAST coletando identificadores não-ligados
//! (free vars) contra um conjunto de bindings locais. `collect_pattern_binds`
//! coleta nomes ligados por patterns.

use std::collections::HashSet;

use kata_ast::Spanned;

use crate::typed::{FusedStage, TypedExpr, TypedExprKind, TypedPattern};

/// Coleta free variables de uma expressão TAST contra um conjunto de
/// bindings locais. Uma free var é um `Ident` cujo nome não está em
/// `local_bindings` e não começa com `__` (compiler-generated).
pub(crate) fn collect_free_vars(
    expr: &TypedExpr,
    local_bindings: &HashSet<String>,
    out: &mut HashSet<String>,
) {
    match &expr.kind {
        TypedExprKind::Ident { name } => {
            if !local_bindings.contains(name) && !name.starts_with("__") {
                out.insert(name.clone());
            }
        }
        TypedExprKind::Closure { callee, args, .. } => {
            // As captures do callee (se Lambda) são free vars do escopo atual.
            // Closure não tem mais campo captures — lê do Lambda interno.
            if let TypedExprKind::Lambda { captures, .. } = &callee.node.kind {
                for cap in captures {
                    if !local_bindings.contains(&cap.name) {
                        out.insert(cap.name.clone());
                    }
                }
            }
            collect_free_vars(&callee.node, local_bindings, out);
            for arg in args {
                collect_free_vars(&arg.node, local_bindings, out);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => {
            collect_free_vars(&expr.node, local_bindings, out);
        }
        TypedExprKind::Grouping { inner } => {
            collect_free_vars(&inner.node, local_bindings, out);
        }
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                collect_free_vars(&el.node, local_bindings, out);
            }
        }
        TypedExprKind::Let { name: _, value } | TypedExprKind::Var { name: _, value } => {
            // value é avaliado antes do binding — free vars do value
            collect_free_vars(&value.node, local_bindings, out);
            // name vira local para as expressões seguintes — MAS como
            // estamos coletando free vars do body de um lambda, o name
            // já está em local_bindings (foi adicionado via pattern binds).
        }
        TypedExprKind::Reassign { value, .. } => {
            collect_free_vars(&value.node, local_bindings, out);
        }
        TypedExprKind::Return(inner) => {
            collect_free_vars(&inner.node, local_bindings, out);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            collect_free_vars(&scrutinee.node, local_bindings, out);
            for arm in arms {
                // Pattern binds são locais para o arm — não propagam free vars
                // Pattern não tem free vars (só literals/sub-patterns)
                if let Some(guard) = &arm.guard {
                    collect_free_vars(&guard.node, local_bindings, out);
                }
                collect_free_vars(&arm.body.node, local_bindings, out);
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            // Lambda aninhada: suas free vars são free vars do escopo atual
            for clause in clauses {
                let mut inner_locals = local_bindings.clone();
                collect_pattern_binds(&clause.patterns, &mut inner_locals);
                for wb in &clause.with_bindings {
                    inner_locals.insert(wb.name.clone());
                }
                if clause.guards.is_empty() {
                    collect_free_vars(&clause.body.node, &inner_locals, out);
                } else {
                    for guard in &clause.guards {
                        if let Some(cond) = &guard.condition {
                            collect_free_vars(&cond.node, &inner_locals, out);
                        }
                        collect_free_vars(&guard.body.node, &inner_locals, out);
                    }
                }
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                collect_free_vars(&stmt.node, local_bindings, out);
            }
        }
        TypedExprKind::ActionCall { args, .. } => {
            collect_free_vars(&args.node, local_bindings, out);
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for val in values {
                collect_free_vars(&val.node, local_bindings, out);
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => {
            collect_free_vars(&expr.node, local_bindings, out);
        }
        TypedExprKind::IndexAccess { expr, .. } => {
            collect_free_vars(&expr.node, local_bindings, out);
        }
        // ── Fio 8: Coleções — recursão nos elementos ──
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                collect_free_vars(&el.node, local_bindings, out);
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            collect_free_vars(&start.node, local_bindings, out);
            collect_free_vars(&step.node, local_bindings, out);
            collect_free_vars(&end.node, local_bindings, out);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            collect_free_vars(&iterable.node, local_bindings, out);
            for stmt in body {
                collect_free_vars(&stmt.node, local_bindings, out);
            }
        }
        TypedExprKind::In { item, collection } => {
            collect_free_vars(&item.node, local_bindings, out);
            collect_free_vars(&collection.node, local_bindings, out);
        }
        // ── Fio 8 Fase 8: map/filter/fold — recursão ──
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
            collect_free_vars(&callback.node, local_bindings, out);
            collect_free_vars(&collection.node, local_bindings, out);
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            collect_free_vars(&callback.node, local_bindings, out);
            collect_free_vars(&initial.node, local_bindings, out);
            collect_free_vars(&collection.node, local_bindings, out);
        }
        // ── Fio 8 Fase 9: FusedStream — recursão ──
        TypedExprKind::FusedStream { stages, source, .. } => {
            collect_free_vars(&source.node, local_bindings, out);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                collect_free_vars(&cb.node, local_bindings, out);
            }
        }
        // ── Fio 11: CSP — recursão ──
        TypedExprKind::ChannelSend { channel, value } => {
            collect_free_vars(&channel.node, local_bindings, out);
            collect_free_vars(&value.node, local_bindings, out);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            collect_free_vars(&channel.node, local_bindings, out);
        }
        TypedExprKind::Select { arms, timeout_ms, timeout_body } => {
            for arm in arms {
                collect_free_vars(&arm.channel.node, local_bindings, out);
                collect_free_vars(&arm.body.node, local_bindings, out);
            }
            if let Some(tm) = timeout_ms {
                collect_free_vars(&tm.node, local_bindings, out);
            }
            if let Some(tb) = timeout_body {
                collect_free_vars(&tb.node, local_bindings, out);
            }
        }
        TypedExprKind::ChannelCreate { .. } => {}
        TypedExprKind::Fork { args, .. } => {
            collect_free_vars(&args.node, local_bindings, out);
        }
        // Folhas sem sub-expressões
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::VariantConstruct { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
    }
}

/// Coleta nomes ligados por patterns (Ident patterns).
pub(crate) fn collect_pattern_binds(patterns: &[Spanned<TypedPattern>], out: &mut HashSet<String>) {
    for pattern in patterns {
        collect_pattern_binds_one(&pattern.node, out);
    }
}

/// Coleta binds de um pattern recursivamente.
pub(crate) fn collect_pattern_binds_one(pattern: &TypedPattern, out: &mut HashSet<String>) {
    match pattern {
        TypedPattern::Ident { name, .. } => {
            out.insert(name.clone());
        }
        TypedPattern::Wildcard => {}
        TypedPattern::Literal { .. } => {}
        TypedPattern::Variant {
            sub_patterns: Some(subs),
            ..
        } => {
            for sub in subs {
                collect_pattern_binds_one(&sub.node, out);
            }
        }
        TypedPattern::Variant { .. } => {}
        TypedPattern::Tuple { elements } => {
            for el in elements {
                collect_pattern_binds_one(&el.node, out);
            }
        }
        TypedPattern::Cons { head, tail } => {
            collect_pattern_binds_one(&head.node, out);
            collect_pattern_binds_one(&tail.node, out);
        }
    }
}
