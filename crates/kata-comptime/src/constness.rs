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
//! - Definições de função do módulo (Ident com ty: Function)
//!
//! **Não comptime-available:**
//! - Parâmetros de função
//! - `var` de Action
//! - `let` bindings cujo initializer não é comptime-available
//! - Qualquer valor que depende de runtime I/O

use kata_core::ty::Ty;
use kata_inference::{TypedExpr, TypedExprKind};

/// Verifica se uma expressão é comptime-available.
pub fn is_comptime_available(expr: &TypedExpr) -> bool {
    check(expr)
}

fn check(expr: &TypedExpr) -> bool {
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
        TypedExprKind::Ident { name: _ } => {
            // Se é uma função nomeada do módulo, é comptime-available.
            if matches!(&expr.ty, Ty::Function(..)) {
                return true;
            }
            // Se é uma Action, NÃO é comptime-available (impuro).
            if matches!(&expr.ty, Ty::Action(..)) {
                return false;
            }
            // TODO: dataflow — verificar se o binding é comptime-available.
            // Por ora, ser conservador: Ident que não é função → NÃO comptime.
            // Na Fase 1, só literais e chamadas de função pura com literais
            // são comptime-available. Bindings serão suportados quando
            // o dataflow for implementado.
            false
        }

        // Closure (chamada de função) — comptime-available se callee e
        // todos os args são comptime-available.
        TypedExprKind::Closure { callee, args, .. } => {
            check(&callee.node) && args.iter().all(|a| check(&a.node))
        }

        // TypeAscription — comptime-available se inner é.
        TypedExprKind::TypeAscription { expr, .. } => check(&expr.node),

        // Grouping — transparente.
        TypedExprKind::Grouping { inner } => check(&inner.node),

        // Tuple — comptime-available se todos elementos são.
        TypedExprKind::Tuple { elements } => elements.iter().all(|e| check(&e.node)),

        // StructConstruct — comptime-available se todos valores são.
        TypedExprKind::StructConstruct { values, .. } => values.iter().all(|v| check(&v.node)),

        // VariantConstruct — comptime se payload é.
        TypedExprKind::VariantConstruct { payload, .. } => check(&payload.node),

        // Let — comptime-available se value é.
        TypedExprKind::Let { value, .. } => check(&value.node),

        // Comptime wrapper — sempre comptime-available (será avaliado).
        TypedExprKind::Comptime { expr } => check(&expr.node),

        // ListLit, ArrayLit — comptime se todos elementos são.
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            elements.iter().all(|e| check(&e.node))
        }

        // RangeLit — comptime se start, step, end são.
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => check(&start.node) && check(&step.node) && check(&end.node),

        // Match — comptime se scrutinee e todos arms body são.
        TypedExprKind::Match { scrutinee, arms } => {
            check(&scrutinee.node)
                && arms.iter().all(|arm| {
                    let guard_ok = match &arm.guard {
                        Some(g) => check(&g.node),
                        None => true,
                    };
                    guard_ok && check(&arm.body.node)
                })
        }

        // Lambda — a definição existe em compile-time. É comptime-available
        // se as cláusulas não referenciam bindings não-comptime.
        TypedExprKind::Lambda { clauses, .. } => clauses.iter().all(|clause| {
            let guards_ok = clause.guards.iter().all(|g| {
                let cond_ok = match &g.condition {
                    Some(c) => check(&c.node),
                    None => true,
                };
                cond_ok && check(&g.body.node)
            });
            let wb_ok = clause.with_bindings.iter().all(|wb| check(&wb.value.node));
            let body_ok = if clause.guards.is_empty() {
                check(&clause.body.node)
            } else {
                true
            };
            guards_ok && wb_ok && body_ok
        }),

        // FieldAccess — comptime se expr é.
        TypedExprKind::FieldAccess { expr, .. } => check(&expr.node),

        // IndexAccess — comptime se expr é.
        TypedExprKind::IndexAccess { expr, .. } => check(&expr.node),

        // Everything else — NÃO comptime-available por padrão.
        _ => false,
    }
}
