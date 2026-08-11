//! Detecção de tail call na TAST — análise pura sobre `TypedExpr`.
//!
//! Extrato de `timer.rs`: não depende de `LowerCtx` nem de Cranelift IR.
//! Walk recursivo procurando `Closure { tail_pos: true, ffi_symbol: None }`.

use kata_inference::{TypedExpr, TypedExprKind, TypedLambdaClause};

/// Verifica se a função faz `return_call` (recursão de cauda).
///
/// Walk recursivo na TAST procurando `Closure { tail_pos: true,
/// ffi_symbol: None }` em qualquer cláusula. Se encontrado, a função
/// faz tail call — o stack slot do `@timer` seria sobrescrito a cada
/// iteração, e o delta medido seria ~0.
///
/// `ffi_symbol: None` garante que não flaggeia chamadas FFI (que não
/// são `return_call` — são `call` comum).
pub(crate) fn has_tail_pos_call(clauses: &[TypedLambdaClause]) -> bool {
    for clause in clauses {
        // Guards: cada guard pode ter um body com tail call.
        for guard in &clause.guards {
            if expr_has_tail_call(&guard.body.node) {
                return true;
            }
        }
        // Corpo da cláusula.
        if expr_has_tail_call(&clause.body.node) {
            return true;
        }
    }
    false
}

/// Walk recursivo em `TypedExpr` procurando `Closure { tail_pos: true,
/// ffi_symbol: None }`.
fn expr_has_tail_call(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            // Se este nó é um tail call não-FFI, achamos.
            if expr.tail_pos && ffi_symbol.is_none() {
                return true;
            }
            // Recursão nos filhos.
            if expr_has_tail_call(&callee.node) {
                return true;
            }
            for arg in args {
                if expr_has_tail_call(&arg.node) {
                    return true;
                }
            }
            false
        }
        TypedExprKind::TypeAscription { expr, .. } => expr_has_tail_call(&expr.node),
        TypedExprKind::Grouping { inner } => expr_has_tail_call(&inner.node),
        TypedExprKind::Let { value, .. } => expr_has_tail_call(&value.node),
        TypedExprKind::LetDestruct {
            value, bindings, ..
        } => {
            if expr_has_tail_call(&value.node) {
                return true;
            }
            bindings
                .iter()
                .any(|(_, expr)| expr_has_tail_call(&expr.node))
        }
        TypedExprKind::Var { value, .. } => expr_has_tail_call(&value.node),
        TypedExprKind::Reassign { value, .. } => expr_has_tail_call(&value.node),
        TypedExprKind::Return(expr) => expr_has_tail_call(&expr.node),
        TypedExprKind::Tuple { elements } => elements.iter().any(|e| expr_has_tail_call(&e.node)),
        TypedExprKind::StructConstruct { values, .. } => {
            values.iter().any(|v| expr_has_tail_call(&v.node))
        }
        TypedExprKind::FieldAccess { expr, .. } => expr_has_tail_call(&expr.node),
        TypedExprKind::IndexAccess { expr, .. } => expr_has_tail_call(&expr.node),
        TypedExprKind::ListLit { elements } => elements.iter().any(|e| expr_has_tail_call(&e.node)),
        TypedExprKind::ArrayLit { elements } => {
            elements.iter().any(|e| expr_has_tail_call(&e.node))
        }
        TypedExprKind::DictLit { entries, .. } => entries
            .iter()
            .any(|(k, v)| expr_has_tail_call(&k.node) || expr_has_tail_call(&v.node)),
        TypedExprKind::SetLit { elements, .. } => {
            elements.iter().any(|e| expr_has_tail_call(&e.node))
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            expr_has_tail_call(&start.node)
                || expr_has_tail_call(&step.node)
                || expr_has_tail_call(&end.node)
        }
        TypedExprKind::Block { stmts } => stmts.iter().any(|s| expr_has_tail_call(&s.node)),
        TypedExprKind::Match { scrutinee, arms } => {
            if expr_has_tail_call(&scrutinee.node) {
                return true;
            }
            arms.iter().any(|arm| {
                if let Some(g) = &arm.guard
                    && expr_has_tail_call(&g.node)
                {
                    return true;
                }
                expr_has_tail_call(&arm.body.node)
            })
        }
        TypedExprKind::ActionCall { args, .. } => expr_has_tail_call(&args.node),
        TypedExprKind::TypeOf { expr } => expr_has_tail_call(&expr.node),
        TypedExprKind::ForIn { iterable, body, .. } => {
            if expr_has_tail_call(&iterable.node) {
                return true;
            }
            body.iter().any(|s| expr_has_tail_call(&s.node))
        }
        TypedExprKind::In { item, collection } => {
            expr_has_tail_call(&item.node) || expr_has_tail_call(&collection.node)
        }
        TypedExprKind::Map {
            callback,
            collection,
            ..
        } => expr_has_tail_call(&callback.node) || expr_has_tail_call(&collection.node),
        TypedExprKind::Filter {
            callback,
            collection,
            ..
        } => expr_has_tail_call(&callback.node) || expr_has_tail_call(&collection.node),
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            expr_has_tail_call(&callback.node)
                || expr_has_tail_call(&initial.node)
                || expr_has_tail_call(&collection.node)
        }
        TypedExprKind::FusedStream { source, .. } => expr_has_tail_call(&source.node),
        TypedExprKind::ChannelSend { channel, value } => {
            expr_has_tail_call(&channel.node) || expr_has_tail_call(&value.node)
        }
        TypedExprKind::ChannelRecv { channel, .. } => expr_has_tail_call(&channel.node),
        TypedExprKind::ChannelCreate { .. } => false,
        TypedExprKind::ReceiverFactoryCall { factory, .. } => expr_has_tail_call(&factory.node),
        TypedExprKind::Fork {
            action_expr, args, ..
        } => expr_has_tail_call(&action_expr.node) || expr_has_tail_call(&args.node),
        TypedExprKind::Spawn {
            action_expr, args, ..
        } => expr_has_tail_call(&action_expr.node) || expr_has_tail_call(&args.node),
        TypedExprKind::Comptime { expr } => expr_has_tail_call(&expr.node),
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            arms.iter().any(|arm| match arm {
                kata_inference::TypedSelectArm::Channel { channel, body, .. } => {
                    expr_has_tail_call(&channel.node) || expr_has_tail_call(&body.node)
                }
                kata_inference::TypedSelectArm::IoRead {
                    handle_expr, body, ..
                } => expr_has_tail_call(&handle_expr.node) || expr_has_tail_call(&body.node),
            }) || timeout_ms
                .as_ref()
                .is_some_and(|t| expr_has_tail_call(&t.node))
                || timeout_body
                    .as_ref()
                    .is_some_and(|t| expr_has_tail_call(&t.node))
        }
        // VariantConstruct tem payload recursivo.
        TypedExprKind::VariantConstruct { payload, .. } => expr_has_tail_call(&payload.node),
        // Loop — body pode conter tail call (break com valor).
        TypedExprKind::Loop { body } => body.iter().any(|s| expr_has_tail_call(&s.node)),
        // Folhas — sem filhos para recursão.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::BytesLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue
        | TypedExprKind::HeapSnapshot { .. } => false,
        // ConstantBinding — não tem tail call (comptime avalia).
        TypedExprKind::ConstantBinding { value, .. } => expr_has_tail_call(&value.node),
        // Lambda — não desce em corpos de lambdas internos (escopo diferente).
        TypedExprKind::Lambda { .. } => false,
    }
}
