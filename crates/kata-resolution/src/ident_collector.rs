//! Coletor de identificadores — walker recursivo sobre a AST.
//!
//! Extraído de `module_loader.rs` (Passo 6, zeladoria). Responsabilidade
//! única: coletar todos os `Ident { name }` de uma expressão/cláusula, para
//! `filter_exports` descobrir dependências internas de corpos de funções
//! exportadas.
//!
//! Visitor puro: só lê a AST e acumula em um `HashSet<String>` — não
//! conhece o loader, cache, ou resolução de paths.

use std::collections::HashSet;

use kata_ast::{DotIndex, Expr, GuardClause, LambdaClause, ReadMode, SelectArm, Spanned};

/// Walker recursivo: coleta todos os `Ident { name }` em uma expressão.
/// Usado para descobrir dependências internas de corpos de funções exportadas.
fn collect_idents(expr: &Spanned<Expr>, out: &mut HashSet<String>) {
    match &expr.node {
        Expr::Ident { name } => {
            out.insert(name.clone());
        }
        Expr::Apply { callee, args } => {
            collect_idents(callee, out);
            for arg in args {
                collect_idents(arg, out);
            }
        }
        Expr::TypeAscription { expr, .. } => collect_idents(expr, out),
        Expr::Grouping { inner } => collect_idents(inner, out),
        Expr::Tuple { elements } => {
            for el in elements {
                collect_idents(el, out);
            }
        }
        Expr::Let { value, .. } => collect_idents(value, out),
        Expr::LetDestruct { value, .. } => collect_idents(value, out),
        Expr::VariantQual { .. } => {}
        Expr::Lambda {
            body,
            guards,
            with_bindings,
            ..
        } => {
            collect_idents(body, out);
            for g in guards {
                collect_guard_idents(g, out);
            }
            for wb in with_bindings {
                collect_idents(&wb.value, out);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_idents(scrutinee, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_idents(g, out);
                }
                collect_idents(&arm.body, out);
            }
        }
        Expr::Hole => {}
        Expr::Pipe { lhs, rhs } => {
            collect_idents(lhs, out);
            collect_idents(rhs, out);
        }
        Expr::PipeLimit { lhs, rhs, limit } => {
            collect_idents(lhs, out);
            collect_idents(rhs, out);
            collect_idents(limit, out);
        }
        Expr::ActionCall { args, .. } => collect_idents(args, out),
        Expr::TypeOf { expr } => collect_idents(expr, out),
        Expr::Return(expr) => collect_idents(expr, out),
        Expr::Loop { body } => {
            for stmt in body {
                collect_idents(stmt, out);
            }
        }
        Expr::Break | Expr::Continue => {}
        Expr::Var { value, .. } => collect_idents(value, out),
        Expr::Reassign { value, .. } => collect_idents(value, out),
        Expr::Question(expr) => collect_idents(expr, out),
        Expr::PipeFallback { lhs, rhs } => {
            collect_idents(lhs, out);
            collect_idents(rhs, out);
        }
        Expr::DotAccess { expr, index } => {
            collect_idents(expr, out);
            if let DotIndex::Range { start, end, .. } = index {
                collect_idents(start, out);
                collect_idents(end, out);
            }
        }
        Expr::ListLit { elements } => {
            for el in elements {
                collect_idents(el, out);
            }
        }
        Expr::ArrayLit { elements } => {
            for el in elements {
                collect_idents(el, out);
            }
        }
        Expr::DictLit { entries } => {
            for (k, v) in entries {
                collect_idents(k, out);
                collect_idents(v, out);
            }
        }
        Expr::SetLit { elements } => {
            for el in elements {
                collect_idents(el, out);
            }
        }
        Expr::RangeLit {
            start, step, end, ..
        } => {
            collect_idents(start, out);
            collect_idents(step, out);
            collect_idents(end, out);
        }
        Expr::ForIn { iterable, body, .. } => {
            collect_idents(iterable, out);
            for stmt in body {
                collect_idents(stmt, out);
            }
        }
        Expr::In { item, collection } => {
            collect_idents(item, out);
            collect_idents(collection, out);
        }
        Expr::ChannelSend { channel, value } => {
            collect_idents(channel, out);
            collect_idents(value, out);
        }
        Expr::ChannelRecv { channel, .. } => collect_idents(channel, out),
        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            for arm in arms {
                collect_select_arm_idents(arm, out);
            }
            if let Some(t) = timeout_ms {
                collect_idents(t, out);
            }
            if let Some(t) = timeout_body {
                collect_idents(t, out);
            }
        }
        Expr::Block { stmts } => {
            for stmt in stmts {
                collect_idents(stmt, out);
            }
        }
        // Literais não contêm idents.
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::TextLit { .. }
        | Expr::BytesLit { .. }
        | Expr::Unit => {}
    }
}

fn collect_guard_idents(guard: &GuardClause, out: &mut HashSet<String>) {
    if let Some(cond) = &guard.condition {
        collect_idents(cond, out);
    }
    collect_idents(&guard.body, out);
}

fn collect_select_arm_idents(arm: &SelectArm, out: &mut HashSet<String>) {
    match arm {
        SelectArm::Channel { channel, body, .. } => {
            collect_idents(channel, out);
            collect_idents(body, out);
        }
        SelectArm::IoRead {
            handle_expr,
            read_mode,
            body,
            ..
        } => {
            collect_idents(handle_expr, out);
            if let ReadMode::Chunk(n) = read_mode {
                collect_idents(n, out);
            }
            collect_idents(body, out);
        }
    }
}

/// Walker sobre `LambdaClause` — coleta idents do body, guards e with_bindings.
pub(crate) fn collect_clause_idents(clause: &Spanned<LambdaClause>, out: &mut HashSet<String>) {
    collect_idents(&clause.node.body, out);
    for g in &clause.node.guards {
        collect_guard_idents(g, out);
    }
    for wb in &clause.node.with_bindings {
        collect_idents(&wb.value, out);
    }
}
