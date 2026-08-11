//! Walker imutável da TAST — percorre sub-expressões de um `&TypedExpr`.
//!
//! `for_each_subexpr` desce recursivamente nos filhos de cada nó, chamando
//! `f` em pré-ordem. Se `f` retorna `false`, a descida nos filhos desse nó
//! é abortada.

use crate::typed::FusedStage;
use crate::typed::{TypedExpr, TypedExprKind, TypedReadMode, TypedSelectArm};
use crate::typed_pattern::{TypedLambdaClause, TypedMatchArm, TypedPattern};

/// Percorre todas as sub-expressões de `expr` em pré-ordem.
///
/// Se `f` retorna `false`, a descida nos filhos desse nó é abortada.
/// Se retorna `true`, a descida continua normalmente.
pub(crate) fn for_each_subexpr<F>(expr: &TypedExpr, f: &mut F)
where
    F: FnMut(&TypedExpr) -> bool,
{
    if f(expr) {
        descend(expr, f);
    }
}

/// Desce nos filhos de `expr` (sem chamar `f` no próprio `expr`).
fn descend<F>(expr: &TypedExpr, f: &mut F)
where
    F: FnMut(&TypedExpr) -> bool,
{
    match &expr.kind {
        TypedExprKind::ActionCall {
            args,
            indirect_callee,
            ..
        } => {
            if let Some(ic) = indirect_callee {
                for_each_subexpr(&ic.node, f);
            }
            for_each_subexpr(&args.node, f);
        }
        TypedExprKind::Closure { callee, args, .. } => {
            for_each_subexpr(&callee.node, f);
            for arg in args {
                for_each_subexpr(&arg.node, f);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => for_each_subexpr(&expr.node, f),
        TypedExprKind::TypeOf { expr } => for_each_subexpr(&expr.node, f),
        TypedExprKind::Grouping { inner } => for_each_subexpr(&inner.node, f),
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                for_each_subexpr(&el.node, f);
            }
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for val in values {
                for_each_subexpr(&val.node, f);
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => for_each_subexpr(&expr.node, f),
        TypedExprKind::IndexAccess { expr, .. } => for_each_subexpr(&expr.node, f),
        TypedExprKind::Let { value, .. }
        | TypedExprKind::LetDestruct { value, .. }
        | TypedExprKind::Var { value, .. } => for_each_subexpr(&value.node, f),
        TypedExprKind::Reassign { value, .. } => for_each_subexpr(&value.node, f),
        TypedExprKind::Return(inner) => for_each_subexpr(&inner.node, f),
        TypedExprKind::VariantConstruct { payload, .. } => {
            for_each_subexpr(&payload.node, f);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            for_each_subexpr(&scrutinee.node, f);
            for arm in arms {
                visit_match_arm(arm, f);
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses {
                visit_lambda_clause(clause, f);
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                for_each_subexpr(&stmt.node, f);
            }
        }
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                for_each_subexpr(&el.node, f);
            }
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (key, val) in entries {
                for_each_subexpr(&key.node, f);
                for_each_subexpr(&val.node, f);
            }
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements {
                for_each_subexpr(&el.node, f);
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            for_each_subexpr(&start.node, f);
            for_each_subexpr(&step.node, f);
            for_each_subexpr(&end.node, f);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            for_each_subexpr(&iterable.node, f);
            for stmt in body {
                for_each_subexpr(&stmt.node, f);
            }
        }
        TypedExprKind::In { item, collection } => {
            for_each_subexpr(&item.node, f);
            for_each_subexpr(&collection.node, f);
        }
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
            for_each_subexpr(&callback.node, f);
            for_each_subexpr(&collection.node, f);
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            for_each_subexpr(&callback.node, f);
            for_each_subexpr(&initial.node, f);
            for_each_subexpr(&collection.node, f);
        }
        TypedExprKind::FusedStream { stages, source, .. } => {
            for_each_subexpr(&source.node, f);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                for_each_subexpr(&cb.node, f);
            }
        }
        TypedExprKind::ChannelSend { channel, value } => {
            for_each_subexpr(&channel.node, f);
            for_each_subexpr(&value.node, f);
        }
        TypedExprKind::ChannelRecv { channel, .. } => for_each_subexpr(&channel.node, f),
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
            ..
        } => {
            for arm in arms {
                match arm {
                    TypedSelectArm::Channel { channel, body, .. } => {
                        for_each_subexpr(&channel.node, f);
                        for_each_subexpr(&body.node, f);
                    }
                    TypedSelectArm::IoRead {
                        handle_expr,
                        read_mode,
                        body,
                        ..
                    } => {
                        for_each_subexpr(&handle_expr.node, f);
                        if let TypedReadMode::Chunk(chunk_size_expr) = read_mode {
                            for_each_subexpr(&chunk_size_expr.node, f);
                        }
                        for_each_subexpr(&body.node, f);
                    }
                }
            }
            if let Some(tm) = timeout_ms {
                for_each_subexpr(&tm.node, f);
            }
            if let Some(tb) = timeout_body {
                for_each_subexpr(&tb.node, f);
            }
        }
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            for_each_subexpr(&factory.node, f);
        }
        TypedExprKind::Fork {
            action_expr, args, ..
        } => {
            for_each_subexpr(&action_expr.node, f);
            for_each_subexpr(&args.node, f);
        }
        TypedExprKind::Spawn {
            action_expr, args, ..
        } => {
            for_each_subexpr(&action_expr.node, f);
            for_each_subexpr(&args.node, f);
        }

        TypedExprKind::Block { stmts } => {
            for stmt in stmts {
                for_each_subexpr(&stmt.node, f);
            }
        }
        TypedExprKind::ConstantBinding { value, .. } => {
            for_each_subexpr(&value.node, f);
        }
        TypedExprKind::HeapSnapshot { .. }
        | TypedExprKind::ChannelCreate { .. }
        | TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::BytesLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
    }
}

fn visit_match_arm<F>(arm: &TypedMatchArm, f: &mut F)
where
    F: FnMut(&TypedExpr) -> bool,
{
    if let Some(pattern) = &arm.pattern {
        visit_pattern(&pattern.node, f);
    }
    if let Some(guard) = &arm.guard {
        for_each_subexpr(&guard.node, f);
    }
    for_each_subexpr(&arm.body.node, f);
}

fn visit_lambda_clause<F>(clause: &TypedLambdaClause, f: &mut F)
where
    F: FnMut(&TypedExpr) -> bool,
{
    for guard in &clause.guards {
        if let Some(cond) = &guard.condition {
            for_each_subexpr(&cond.node, f);
        }
        for_each_subexpr(&guard.body.node, f);
    }
    for wb in &clause.with_bindings {
        for_each_subexpr(&wb.value.node, f);
    }
    if clause.guards.is_empty() {
        for_each_subexpr(&clause.body.node, f);
    }
}

fn visit_pattern<F>(pattern: &TypedPattern, f: &mut F)
where
    F: FnMut(&TypedExpr) -> bool,
{
    match pattern {
        TypedPattern::Literal { value } => for_each_subexpr(&value.node, f),
        TypedPattern::Variant {
            sub_patterns: Some(subs),
            ..
        } => {
            for sub in subs {
                visit_pattern(&sub.node, f);
            }
        }
        TypedPattern::Tuple { elements } => {
            for el in elements {
                visit_pattern(&el.node, f);
            }
        }
        TypedPattern::Cons { head, tail } => {
            visit_pattern(&head.node, f);
            visit_pattern(&tail.node, f);
        }
        TypedPattern::Ident { .. }
        | TypedPattern::Wildcard
        | TypedPattern::Variant {
            sub_patterns: None, ..
        }
        | TypedPattern::Nil => {}
    }
}
