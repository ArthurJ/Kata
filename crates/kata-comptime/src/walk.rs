//! Traversal recursivo sobre `TypedExpr` — versões mut e ref, mais
//! detector de nós `Comptime`.

use kata_inference::{TypedExpr, TypedExprKind};

use crate::error::ComptimeError;

/// Walk mut nos filhos de `expr` (não no próprio expr).
pub(crate) fn walk_mut<F>(expr: &mut TypedExpr, f: &mut F) -> Result<(), ComptimeError>
where
    F: FnMut(&mut TypedExpr) -> Result<(), ComptimeError>,
{
    match &mut expr.kind {
        TypedExprKind::Let { value, .. } | TypedExprKind::Var { value, .. } => {
            f(&mut value.node)?;
        }
        TypedExprKind::LetDestruct {
            value, bindings, ..
        } => {
            f(&mut value.node)?;
            for (_, b) in bindings.iter_mut() {
                f(&mut b.node)?;
            }
        }
        TypedExprKind::Closure { callee, args, .. } => {
            f(&mut callee.node)?;
            for arg in args.iter_mut() {
                f(&mut arg.node)?;
            }
        }
        TypedExprKind::Grouping { inner } => f(&mut inner.node)?,
        TypedExprKind::Tuple { elements } => {
            for el in elements.iter_mut() {
                f(&mut el.node)?;
            }
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for v in values.iter_mut() {
                f(&mut v.node)?;
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => f(&mut expr.node)?,
        TypedExprKind::IndexAccess { expr, .. } => f(&mut expr.node)?,
        TypedExprKind::TypeAscription { expr, .. } => f(&mut expr.node)?,
        TypedExprKind::TypeOf { expr } => f(&mut expr.node)?,
        TypedExprKind::Comptime { expr } => f(&mut expr.node)?,
        TypedExprKind::Match { scrutinee, arms } => {
            f(&mut scrutinee.node)?;
            for arm in arms.iter_mut() {
                if let Some(guard) = &mut arm.guard {
                    f(&mut guard.node)?;
                }
                f(&mut arm.body.node)?;
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses.iter_mut() {
                for guard in &mut clause.guards {
                    if let Some(cond) = &mut guard.condition {
                        f(&mut cond.node)?;
                    }
                    f(&mut guard.body.node)?;
                }
                for wb in &mut clause.with_bindings {
                    f(&mut wb.value.node)?;
                }
                if clause.guards.is_empty() {
                    f(&mut clause.body.node)?;
                }
            }
        }
        TypedExprKind::Return(inner) => f(&mut inner.node)?,
        TypedExprKind::Reassign { value, .. } => f(&mut value.node)?,
        TypedExprKind::Loop { body } => {
            for stmt in body.iter_mut() {
                f(&mut stmt.node)?;
            }
        }
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements.iter_mut() {
                f(&mut el.node)?;
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            f(&mut start.node)?;
            f(&mut step.node)?;
            f(&mut end.node)?;
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            f(&mut iterable.node)?;
            for stmt in body.iter_mut() {
                f(&mut stmt.node)?;
            }
        }
        // HeapSnapshot — folha.
        TypedExprKind::HeapSnapshot { .. } => {}
        // ActionCall: percorre args (Box<Spanned<TypedExpr>>).
        TypedExprKind::ActionCall { args, .. } => {
            f(&mut args.node)?;
        }
        // ConstantBinding: percorre value.
        TypedExprKind::ConstantBinding { value, .. } => {
            f(&mut value.node)?;
        }
        // Outros variants não têm filhos TypedExpr ou não aparecem em top-level.
        _ => {}
    }
    Ok(())
}

/// Verifica se a expressão contém algum nó `Comptime`.
pub(crate) fn contains_comptime(expr: &TypedExpr) -> bool {
    let mut found = false;
    walk_ref(expr, &mut |e| {
        if matches!(e.kind, TypedExprKind::Comptime { .. }) {
            found = true;
        }
    });
    found
}

/// Walk imutável nos filhos de `expr`.
pub(crate) fn walk_ref<F: FnMut(&TypedExpr)>(expr: &TypedExpr, f: &mut F) {
    f(expr);
    match &expr.kind {
        TypedExprKind::Let { value, .. } | TypedExprKind::Var { value, .. } => {
            walk_ref(&value.node, f);
        }
        TypedExprKind::Comptime { expr } => walk_ref(&expr.node, f),
        TypedExprKind::Closure { callee, args, .. } => {
            walk_ref(&callee.node, f);
            for arg in args {
                walk_ref(&arg.node, f);
            }
        }
        TypedExprKind::Grouping { inner } => walk_ref(&inner.node, f),
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                walk_ref(&el.node, f);
            }
        }
        TypedExprKind::HeapSnapshot { .. } => {}
        _ => {}
    }
}
