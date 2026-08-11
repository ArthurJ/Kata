//! Análise de constness — verifica se uma expressão é comptime-available.
//!
//! "Comptime-available" é binário: existe em compile-time ou não.
//! Não há lattice, nem níveis de "quão constante."
//!
//! **Comptime-available:**
//! - Literais (IntLit, FloatLit, TextLit, Unit)
//! - VariantQual (Boolean::True, etc.)
//! - Resultados de @comptime anterior (já são Literals na TAST após substituição)
//! - `let` bindings cujo initializer é comptime-available (propagação dataflow)
//! - Bindings locais avaliados via `@comptime let` (registrados no `comptime_bindings`)
//! - Definições de função do módulo (Ident com ty: Function)
//!
//! **Não comptime-available:**
//! - Parâmetros de função
//! - `var` de Action
//! - `let` bindings cujo initializer não é comptime-available
//! - Qualquer valor que depende de runtime I/O

use std::collections::HashMap;

use kata_core::ty::Ty;
use kata_inference::{TypedExpr, TypedExprKind};

/// Verifica se uma expressão é comptime-available.
///
/// `comptime_bindings` é o mapa de nomes de bindings locais para seus
/// valores literais, avaliados em compile-time (via @comptime let).
/// Um Ident que referencia um binding neste mapa é comptime-available.
pub(crate) fn is_comptime_available(
    expr: &TypedExpr,
    comptime_bindings: &HashMap<String, TypedExpr>,
) -> bool {
    check(expr, comptime_bindings)
}

fn check(expr: &TypedExpr, comptime_bindings: &HashMap<String, TypedExpr>) -> bool {
    match &expr.kind {
        // Literais — sempre comptime-available.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::VariantQual { .. } => true,

        // Ident — pode ser:
        // 1. Referência a função nomeada (ty: Function) — comptime-available
        //    (a definição existe; o comptime pass pode compilar e executar).
        // 2. Binding comptime-available (propagado por dataflow) — comptime.
        // 3. Parâmetro ou var runtime — NÃO comptime-available.
        TypedExprKind::Ident { name } => {
            // Se é uma função nomeada do módulo, é comptime-available.
            if matches!(&expr.ty, Ty::Function(..)) {
                return true;
            }
            // Se é uma Action, NÃO é comptime-available (impuro).
            if matches!(&expr.ty, Ty::Action(..)) {
                return false;
            }
            // Dataflow: se o binding está no conjunto de bindings
            // comptime-available, é comptime.
            if comptime_bindings.contains_key(name) {
                return true;
            }
            false
        }

        // Closure (chamada de função) — comptime-available se callee e
        // todos os args são comptime-available.
        TypedExprKind::Closure { callee, args, .. } => {
            check(&callee.node, comptime_bindings)
                && args.iter().all(|a| check(&a.node, comptime_bindings))
        }

        // TypeAscription — comptime-available se inner é.
        TypedExprKind::TypeAscription { expr, .. } => check(&expr.node, comptime_bindings),

        // Grouping — transparente.
        TypedExprKind::Grouping { inner } => check(&inner.node, comptime_bindings),

        // Tuple — comptime-available se todos elementos são.
        TypedExprKind::Tuple { elements } => {
            elements.iter().all(|e| check(&e.node, comptime_bindings))
        }

        // StructConstruct — comptime-available se todos valores são.
        TypedExprKind::StructConstruct { values, .. } => {
            values.iter().all(|v| check(&v.node, comptime_bindings))
        }

        // VariantConstruct — comptime se payload é.
        TypedExprKind::VariantConstruct { payload, .. } => check(&payload.node, comptime_bindings),

        // Let — comptime-available se value é.
        TypedExprKind::Let { value, .. } => check(&value.node, comptime_bindings),



        // HeapSnapshot — sempre comptime-available (já avaliado).
        TypedExprKind::HeapSnapshot { .. } => true,

        // ListLit, ArrayLit — comptime se todos elementos são.
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            elements.iter().all(|e| check(&e.node, comptime_bindings))
        }

        // RangeLit — comptime se start, step, end são.
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            check(&start.node, comptime_bindings)
                && check(&step.node, comptime_bindings)
                && check(&end.node, comptime_bindings)
        }

        // Match — comptime se scrutinee e todos arms body são.
        TypedExprKind::Match { scrutinee, arms } => {
            check(&scrutinee.node, comptime_bindings)
                && arms.iter().all(|arm| {
                    let guard_ok = match &arm.guard {
                        Some(g) => check(&g.node, comptime_bindings),
                        None => true,
                    };
                    guard_ok && check(&arm.body.node, comptime_bindings)
                })
        }

        // Lambda — a definição existe em compile-time. É comptime-available
        // se as cláusulas não referenciam bindings não-comptime.
        TypedExprKind::Lambda { clauses, .. } => clauses.iter().all(|clause| {
            let guards_ok = clause.guards.iter().all(|g| {
                let cond_ok = match &g.condition {
                    Some(c) => check(&c.node, comptime_bindings),
                    None => true,
                };
                cond_ok && check(&g.body.node, comptime_bindings)
            });
            let wb_ok = clause
                .with_bindings
                .iter()
                .all(|wb| check(&wb.value.node, comptime_bindings));
            let body_ok = if clause.guards.is_empty() {
                check(&clause.body.node, comptime_bindings)
            } else {
                true
            };
            guards_ok && wb_ok && body_ok
        }),

        // FieldAccess — comptime se expr é.
        TypedExprKind::FieldAccess { expr, .. } => check(&expr.node, comptime_bindings),

        // IndexAccess — comptime se expr é.
        TypedExprKind::IndexAccess { expr, .. } => check(&expr.node, comptime_bindings),

        // Everything else — NÃO comptime-available por padrão.
        _ => false,
    }
}
