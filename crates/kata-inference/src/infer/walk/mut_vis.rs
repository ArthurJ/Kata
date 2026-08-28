//! Walker mutável da TAST — percorre sub-expressões de um `&mut TypedExpr`.
//!
//! `for_each_subexpr_mut` desce recursivamente nos filhos de cada nó,
//! chamando `f` em pré-ordem. Se `f` retorna `false`, a descida nos filhos
//! desse nó é abortada.

use crate::typed::FusedStage;
use crate::typed::{TypedExpr, TypedExprKind, TypedReadMode, TypedSelectArm};
use crate::typed_pattern::{TypedLambdaClause, TypedMatchArm, TypedPattern};

/// Percorre todas as sub-expressões de `expr` em pré-ordem (versão mutável).
///
/// Se `f` retorna `false`, a descida nos filhos desse nó é abortada.
/// Se retorna `true`, a descida continua normalmente.
pub(crate) fn for_each_subexpr_mut<F>(expr: &mut TypedExpr, f: &mut F)
where
    F: FnMut(&mut TypedExpr) -> bool,
{
    if f(expr) {
        descend_mut(expr, f);
    }
}

/// Desce nos filhos de `expr` (versão mutável, sem chamar `f` no próprio `expr`).
fn descend_mut<F>(expr: &mut TypedExpr, f: &mut F)
where
    F: FnMut(&mut TypedExpr) -> bool,
{
    match &mut expr.kind {
        TypedExprKind::ActionCall {
            args,
            indirect_callee,
            ..
        } => {
            if let Some(ic) = indirect_callee {
                for_each_subexpr_mut(&mut ic.node, f);
            }
            for_each_subexpr_mut(&mut args.node, f);
        }
        TypedExprKind::Closure { callee, args, .. } => {
            for_each_subexpr_mut(&mut callee.node, f);
            for arg in args {
                for_each_subexpr_mut(&mut arg.node, f);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => for_each_subexpr_mut(&mut expr.node, f),
        TypedExprKind::TypeOf { expr } => for_each_subexpr_mut(&mut expr.node, f),
        TypedExprKind::Grouping { inner } => for_each_subexpr_mut(&mut inner.node, f),
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                for_each_subexpr_mut(&mut el.node, f);
            }
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for val in values {
                for_each_subexpr_mut(&mut val.node, f);
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => for_each_subexpr_mut(&mut expr.node, f),
        TypedExprKind::IndexAccess { expr, .. } => for_each_subexpr_mut(&mut expr.node, f),
        TypedExprKind::Let { value, .. }
        | TypedExprKind::LetDestruct { value, .. }
        | TypedExprKind::Var { value, .. } => for_each_subexpr_mut(&mut value.node, f),
        TypedExprKind::Reassign { value, .. } => for_each_subexpr_mut(&mut value.node, f),
        TypedExprKind::Return(inner) => for_each_subexpr_mut(&mut inner.node, f),
        TypedExprKind::VariantConstruct { payload, .. } => {
            for_each_subexpr_mut(&mut payload.node, f);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            for_each_subexpr_mut(&mut scrutinee.node, f);
            for arm in arms {
                visit_match_arm_mut(arm, f);
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses {
                visit_lambda_clause_mut(clause, f);
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                for_each_subexpr_mut(&mut stmt.node, f);
            }
        }
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                for_each_subexpr_mut(&mut el.node, f);
            }
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (key, val) in entries {
                for_each_subexpr_mut(&mut key.node, f);
                for_each_subexpr_mut(&mut val.node, f);
            }
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements {
                for_each_subexpr_mut(&mut el.node, f);
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            for_each_subexpr_mut(&mut start.node, f);
            for_each_subexpr_mut(&mut step.node, f);
            for_each_subexpr_mut(&mut end.node, f);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            for_each_subexpr_mut(&mut iterable.node, f);
            for stmt in body {
                for_each_subexpr_mut(&mut stmt.node, f);
            }
        }
        TypedExprKind::In { item, collection } => {
            for_each_subexpr_mut(&mut item.node, f);
            for_each_subexpr_mut(&mut collection.node, f);
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
            for_each_subexpr_mut(&mut callback.node, f);
            for_each_subexpr_mut(&mut collection.node, f);
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            for_each_subexpr_mut(&mut callback.node, f);
            for_each_subexpr_mut(&mut initial.node, f);
            for_each_subexpr_mut(&mut collection.node, f);
        }
        TypedExprKind::FusedStream { stages, source, .. } => {
            for_each_subexpr_mut(&mut source.node, f);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                for_each_subexpr_mut(&mut cb.node, f);
            }
        }
        TypedExprKind::ChannelSend { channel, value } => {
            for_each_subexpr_mut(&mut channel.node, f);
            for_each_subexpr_mut(&mut value.node, f);
        }
        TypedExprKind::ChannelRecv { channel, .. } => for_each_subexpr_mut(&mut channel.node, f),
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
            ..
        } => {
            for arm in arms {
                match arm {
                    TypedSelectArm::Channel { channel, body, .. } => {
                        for_each_subexpr_mut(&mut channel.node, f);
                        for_each_subexpr_mut(&mut body.node, f);
                    }
                    TypedSelectArm::IoRead {
                        handle_expr,
                        read_mode,
                        body,
                        ..
                    } => {
                        for_each_subexpr_mut(&mut handle_expr.node, f);
                        if let TypedReadMode::Chunk(chunk_size_expr) = read_mode {
                            for_each_subexpr_mut(&mut chunk_size_expr.node, f);
                        }
                        for_each_subexpr_mut(&mut body.node, f);
                    }
                }
            }
            if let Some(tm) = timeout_ms {
                for_each_subexpr_mut(&mut tm.node, f);
            }
            if let Some(tb) = timeout_body {
                for_each_subexpr_mut(&mut tb.node, f);
            }
        }
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            for_each_subexpr_mut(&mut factory.node, f);
        }
        TypedExprKind::Fork {
            action_expr, args, ..
        } => {
            for_each_subexpr_mut(&mut action_expr.node, f);
            for_each_subexpr_mut(&mut args.node, f);
        }
        TypedExprKind::Spawn {
            action_expr, args, ..
        } => {
            for_each_subexpr_mut(&mut action_expr.node, f);
            for_each_subexpr_mut(&mut args.node, f);
        }

        TypedExprKind::Block { stmts } => {
            for stmt in stmts {
                for_each_subexpr_mut(&mut stmt.node, f);
            }
        }
        TypedExprKind::ConstantBinding { value, .. } => {
            for_each_subexpr_mut(&mut value.node, f);
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

fn visit_match_arm_mut<F>(arm: &mut TypedMatchArm, f: &mut F)
where
    F: FnMut(&mut TypedExpr) -> bool,
{
    if let Some(pattern) = &mut arm.pattern {
        visit_pattern_mut(&mut pattern.node, f);
    }
    if let Some(guard) = &mut arm.guard {
        for_each_subexpr_mut(&mut guard.node, f);
    }
    for_each_subexpr_mut(&mut arm.body.node, f);
}

fn visit_lambda_clause_mut<F>(clause: &mut TypedLambdaClause, f: &mut F)
where
    F: FnMut(&mut TypedExpr) -> bool,
{
    for guard in &mut clause.guards {
        if let Some(cond) = &mut guard.condition {
            for_each_subexpr_mut(&mut cond.node, f);
        }
        for_each_subexpr_mut(&mut guard.body.node, f);
    }
    for wb in &mut clause.with_bindings {
        for_each_subexpr_mut(&mut wb.value.node, f);
    }
    for pre in &mut clause.synthetic_pre {
        for_each_subexpr_mut(&mut pre.node, f);
    }
    for post in &mut clause.synthetic_post {
        for_each_subexpr_mut(&mut post.node, f);
    }
    if clause.guards.is_empty() {
        for_each_subexpr_mut(&mut clause.body.node, f);
    }
}

fn visit_pattern_mut<F>(pattern: &mut TypedPattern, f: &mut F)
where
    F: FnMut(&mut TypedExpr) -> bool,
{
    match pattern {
        TypedPattern::Literal { value } => for_each_subexpr_mut(&mut value.node, f),
        TypedPattern::Variant {
            sub_patterns: Some(subs),
            ..
        } => {
            for sub in subs {
                visit_pattern_mut(&mut sub.node, f);
            }
        }
        TypedPattern::Tuple { elements } => {
            for el in elements {
                visit_pattern_mut(&mut el.node, f);
            }
        }
        TypedPattern::Cons { head, tail } => {
            visit_pattern_mut(&mut head.node, f);
            visit_pattern_mut(&mut tail.node, f);
        }
        TypedPattern::Ident { .. }
        | TypedPattern::Wildcard
        | TypedPattern::Variant {
            sub_patterns: None, ..
        }
        | TypedPattern::Nil => {}
    }
}
