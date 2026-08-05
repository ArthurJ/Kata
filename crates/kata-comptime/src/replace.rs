//! Substituição de nós `Comptime` por literais/snapshots — o coração do pass.

use std::collections::HashMap;

use kata_ast::Spanned;
use kata_inference::{TypedExpr, TypedExprKind};

use crate::constness::is_comptime_available;
use crate::ctx::ModuleCtx;
use crate::error::ComptimeError;
use crate::jit::jit_execute_expr;
use crate::pureza::check_purity;
use crate::result::result_to_literal;
use crate::walk::walk_mut;

/// Substitui nós `Comptime` recursivamente num `TypedExpr`.
pub(crate) fn replace_comptime_in_place(
    expr: &mut TypedExpr,
    ctx: &ModuleCtx<'_>,
    changed: &mut bool,
    snapshots: &mut Vec<kata_core::snapshot::HeapSnapshotData>,
    comptime_bindings: &mut HashMap<String, TypedExpr>,
) -> Result<(), ComptimeError> {
    if !matches!(expr.kind, TypedExprKind::Comptime { .. }) {
        // Recursão nos filhos.
        walk_mut(expr, &mut |child| {
            replace_comptime_in_place(child, ctx, changed, snapshots, comptime_bindings)
        })?;
        return Ok(());
    }

    // Extrair o inner do Comptime via mem::replace para evitar borrow conflict.
    // Usa Unit como placeholder (será sobrescrito).
    let inner_owned = match std::mem::replace(&mut expr.kind, TypedExprKind::Unit) {
        TypedExprKind::Comptime { expr } => *expr,
        _ => unreachable!(),
    };
    let inner = &inner_owned.node;

    // Caso especial: @comptime envolve um Let. Avaliar apenas o
    // `value` do let e preservar o binding, substituindo o Comptime
    // inteiro por `Let { name, value: <literal> }`.
    if let TypedExprKind::Let { name, value } = &inner.kind {
        // 1. Verificar constness do value (não do let inteiro).
        if !is_comptime_available(&value.node, comptime_bindings) {
            // Restaurar antes de propagar erro.
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(ComptimeError::NotConsttime {
                reason: "expressão depende de valor runtime".into(),
            });
        }

        // 2. Verificar pureza do value.
        if let Err(e) = check_purity(&value.node) {
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(e);
        }

        // 3. JIT-executar o value.
        let result = match jit_execute_expr(&value.node, ctx, comptime_bindings) {
            Ok(r) => r,
            Err(e) => {
                expr.kind = TypedExprKind::Comptime {
                    expr: Box::new(inner_owned),
                };
                return Err(e);
            }
        };

        // 4. Substituir por literal ou HeapSnapshot.
        let literal = match result_to_literal(
            &result,
            &value.node,
            snapshots,
            ctx.struct_registry,
            ctx.enum_registry,
        ) {
            Ok(l) => l,
            Err(e) => {
                expr.kind = TypedExprKind::Comptime {
                    expr: Box::new(inner_owned),
                };
                return Err(e);
            }
        };

        // 5. Reconstruir o Let com o value substituído pelo literal.
        let literal_expr = Spanned::new(literal.clone(), value.span);
        expr.kind = TypedExprKind::Let {
            name: name.clone(),
            value: Box::new(literal_expr),
        };
        expr.ty = inner.ty.clone();
        // 6. Registrar o binding como comptime-available para dataflow.
        comptime_bindings.insert(name.clone(), literal);
        *changed = true;
        return Ok(());
    }

    // Caso geral: @comptime envolve uma expressão qualquer.
    // 1. Verificar constness do inner expr.
    if !is_comptime_available(inner, comptime_bindings) {
        expr.kind = TypedExprKind::Comptime {
            expr: Box::new(inner_owned),
        };
        return Err(ComptimeError::NotConsttime {
            reason: "expressão depende de valor runtime".into(),
        });
    }

    // 2. Verificar pureza.
    if let Err(e) = check_purity(inner) {
        expr.kind = TypedExprKind::Comptime {
            expr: Box::new(inner_owned),
        };
        return Err(e);
    }

    // 3. JIT-executar o inner expr.
    let result = match jit_execute_expr(inner, ctx, comptime_bindings) {
        Ok(r) => r,
        Err(e) => {
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(e);
        }
    };

    // 4. Substituir por literal (escalar) ou HeapSnapshot (complexo).
    let replacement = match result_to_literal(
        &result,
        inner,
        snapshots,
        ctx.struct_registry,
        ctx.enum_registry,
    ) {
        Ok(r) => r,
        Err(e) => {
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(e);
        }
    };

    // 5. Trocar o kind do expr.
    expr.kind = replacement.kind;
    expr.ty = replacement.ty;
    *changed = true;
    Ok(())
}
