//! Validação de predicados complexos pendentes (Fase 4).
//!
//! TypeAscription com pending_predicates foi produzida pelo typeck quando
//! const_eval não conseguiou avaliar (predicado complexo, ex: is_prime).
//! Aqui, JIT-executamos cada predicado e verificamos se retorna Boolean::True.

use std::collections::HashMap;

use kata_inference::{TypedExpr, TypedExprKind};

use crate::ctx::ModuleCtx;
use crate::error::ComptimeError;
use crate::jit::jit_execute_expr;
use crate::walk::walk_mut;
use kata_diagnostics::MietteSpan;

/// Walk recursivo nos filhos de `expr` chamando `validate_pending_predicates`.
/// Quando encontra `TypeAscription` com `pending_predicates` não-vazio,
/// JIT-executa cada predicado e verifica se retorna `Boolean::True`.
pub(crate) fn validate_pending_predicates(
    expr: &mut TypedExpr,
    ctx: &ModuleCtx<'_>,
    comptime_bindings: &HashMap<String, TypedExpr>,
) -> Result<(), ComptimeError> {
    // Primeiro recursar nos filhos.
    walk_mut(expr, &mut |child| {
        validate_pending_predicates(child, ctx, comptime_bindings)
    })?;

    // Depois processar o próprio nó se for TypeAscription com pending.
    if let TypedExprKind::TypeAscription {
        pending_predicates, ..
    } = &mut expr.kind
        && !pending_predicates.is_empty()
    {
        for pred in pending_predicates.iter() {
            let result =
                jit_execute_expr(&pred.node, ctx, comptime_bindings, ctx.functions, &[], &[])?;
            // Resultado deve ser Boolean::True (tag 1) ou Boolean::False (tag 0).
            // O runtime representa Boolean como Sum com tag 0 (False) ou 1 (True).
            if result.raw != 1 {
                return Err(ComptimeError::RefinedViolation {
                    span: MietteSpan(expr.span),
                });
            }
        }
        // Todos os predicados passaram — limpa pending.
        pending_predicates.clear();
    }
    Ok(())
}
