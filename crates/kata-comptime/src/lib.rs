//! Comptime pass — avaliação em compile-time via JIT-and-execute.
//!
//! Percorre a TAST identificando nós `TypedExprKind::Comptime`, verifica
//! constness e pureza, JIT-executa a expressão, e substitui por `Literal`
//! (escalares) ou `HeapSnapshot` (tipos complexos — Fase 2).
//!
//! Posição no pipeline:
//! ```text
//! ... → tree_shake → comptime → lowering → ...
//! ```
//!
//! O pass recebe um `TypedModule` e retorna um `TypedModule` com os nós
//! `Comptime` substituídos. Para a Fase 1, apenas resultados escalares
//! (Int, Float, Boolean, Unit) são suportados — viram `IntLit`/`FloatLit`/
//! `TextLit`/`Unit` directo na TAST.

mod constness;
mod ctx;
mod error;
mod fold;
mod jit;
mod predicates;
mod pureza;
mod replace;
mod result;
mod snapshot;
mod walk;

use std::collections::HashMap;

use kata_core::EnumRegistry;
use kata_inference::{TypedExpr, TypedModule};

use ctx::ModuleCtx;
use fold::fold_literal_calls;
use predicates::validate_pending_predicates;
use replace::replace_comptime_in_place;
use walk::contains_comptime;

// Re-export da API pública.
pub use error::ComptimeError;

/// Serializa um valor JIT-executado em `HeapSnapshotData`.
///
/// Wrapper público sobre `snapshot::serialize_snapshot` (que é `pub(crate)`).
/// Usado pelo REPL para congelar bindings complexos (List, Struct, Tuple,
/// Text, Sum) como snapshots persistidos na root_arena.
pub fn serialize_value(
    raw: i64,
    ty: &kata_core::ty::Ty,
    struct_registry: &kata_core::StructRegistry,
    enum_registry: &kata_core::EnumRegistry,
) -> Result<kata_core::snapshot::HeapSnapshotData, String> {
    snapshot::serialize_snapshot(raw, ty, struct_registry, enum_registry)
}

/// Executa o comptime pass num `TypedModule`.
///
/// Percorre `pre_entry` e `entry` substituindo nós `TypedExprKind::Comptime`
/// por literais (escalares) ou snapshots (complexos — Fase 2).
///
/// Repete até fixpoint (sem novos nós `Comptime`).
pub fn run_comptime_pass(
    typed: TypedModule,
    enum_registry: &EnumRegistry,
) -> Result<TypedModule, ComptimeError> {
    let mut current = typed;
    // Acumulador de snapshots — populado por replace_comptime_in_place.
    // No fim, atribuído a current.snapshots.
    let mut snapshots: Vec<kata_core::snapshot::HeapSnapshotData> =
        std::mem::take(&mut current.snapshots);

    // Bindings comptime-available — construído incrementalmente durante o
    // fixpoint. Após substituir @comptime let x := ..., x → literal é
    // adicionado aqui. Um @comptime posterior que referencia x vê o binding
    // no mapa. O mapa também é injetado no mini TypedModule para o JIT
    // resolver Idents comptime-available.
    let mut comptime_bindings: HashMap<String, TypedExpr> = HashMap::new();

    // Fixpoint: substituir Comptime pode revelar novos Comptime em inner exprs.
    loop {
        let mut changed = false;

        // Clonar actions antes do loop para evitar conflito de borrow:
        // o ctx precisa de &actions (imutável) para jit_execute_expr, mas
        // precisamos mutar current.actions[i].body. Clonar resolve
        // (consistente com jit_execute_expr que já clona tudo).
        let actions_clone = current.actions.clone();

        // Partial borrow: empresta campos imutáveis individuais de `current`.
        // Não conflita com `&mut current.pre_entry` / `&mut current.entry`
        // porque são campos diferentes do mesmo struct.
        let ctx = ModuleCtx {
            dispatch_table: &current.dispatch_table,
            type_env: &current.type_env,
            functions: &current.functions,
            actions: &actions_clone,
            struct_registry: &current.struct_registry,
            enum_registry,
        };

        // Processar pre_entry
        for expr in &mut current.pre_entry {
            let was_comptime = contains_comptime(&expr.node);
            replace_comptime_in_place(
                &mut expr.node,
                &ctx,
                &mut changed,
                &mut snapshots,
                &mut comptime_bindings,
            )?;
            if was_comptime && !contains_comptime(&expr.node) {
                // Já substituído — ok
            }
        }

        // Processar entry
        replace_comptime_in_place(
            &mut current.entry.node,
            &ctx,
            &mut changed,
            &mut snapshots,
            &mut comptime_bindings,
        )?;

        // Processar bodies de actions (Fase 3b)
        for action in &mut current.actions {
            for stmt in &mut action.body {
                replace_comptime_in_place(
                    &mut stmt.node,
                    &ctx,
                    &mut changed,
                    &mut snapshots,
                    &mut comptime_bindings,
                )?;
            }
        }

        // ── Ponto 7: Constant folding de chamadas com args literais ──
        // Após replace_comptime_in_place, percorre a TAST procurando
        // Closures com ffi_symbol: None e todos args literais. JIT-executa
        // e substitui por literal. O fixpoint garante que folds em cascade
        // (o resultado de um fold pode ser arg literal de outro).
        for expr in &mut current.pre_entry {
            fold_literal_calls(
                &mut expr.node,
                &ctx,
                &mut changed,
                &mut snapshots,
                &comptime_bindings,
            )?;
        }
        fold_literal_calls(
            &mut current.entry.node,
            &ctx,
            &mut changed,
            &mut snapshots,
            &comptime_bindings,
        )?;
        for action in &mut current.actions {
            for stmt in &mut action.body {
                fold_literal_calls(
                    &mut stmt.node,
                    &ctx,
                    &mut changed,
                    &mut snapshots,
                    &comptime_bindings,
                )?;
            }
        }

        if !changed {
            break;
        }
    }

    current.snapshots = snapshots;

    // ── Fase 4: Validar predicados complexos pendentes ──
    // TypeAscription com pending_predicates foi produzida pelo typeck quando
    // const_eval não conseguiu avaliar (predicado complexo, ex: is_prime).
    // Aqui, JIT-executamos cada predicado e verificamos se retorna Boolean::True.
    let actions_clone = current.actions.clone();
    let ctx = ModuleCtx {
        dispatch_table: &current.dispatch_table,
        type_env: &current.type_env,
        functions: &current.functions,
        actions: &actions_clone,
        struct_registry: &current.struct_registry,
        enum_registry,
    };
    for expr in &mut current.pre_entry {
        validate_pending_predicates(&mut expr.node, &ctx, &comptime_bindings)?;
    }
    validate_pending_predicates(&mut current.entry.node, &ctx, &comptime_bindings)?;
    for action in &mut current.actions {
        for stmt in &mut action.body {
            validate_pending_predicates(&mut stmt.node, &ctx, &comptime_bindings)?;
        }
    }
    Ok(current)
}
