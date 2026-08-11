//! Comptime pass — avaliação em compile-time via JIT-and-execute.
//!
//! Avalia `ConstantBinding`s (constants de módulo), faz fold de chamadas
//! literais, valida predicados pendentes, e substitui refs a constants
//! nos corpos de functions e actions.
//!
//! Posição no pipeline:
//! ```text
//! ... → tree_shake → comptime → lowering → ...
//! ```
//!
//! O pass recebe um `TypedModule` e retorna um `TypedModule` com as
//! constants avaliadas (literais escalares ou `HeapSnapshot` para tipos
//! complexos) e chamadas literais foldadas.
//!
//! Arquitetura (C2):
//! 1. `evaluate_constants` — pre-pass linear: percorre constants em ordem
//!    de declaração, avalia cada uma uma vez. Sem fixpoint. A ordem de
//!    declaração é a ordem de dependência (a inferência já garante que
//!    forward references falham com `UnboundName`).
//! 2. Fixpoint loop — só para `fold_literal_calls` (cascata de folds:
//!    o resultado de um fold pode ser arg literal de outro).
//! 3. `fold_constant_refs_in_functions/actions` — substitui Idents de
//!    constants nos corpos de functions e actions.
//! 4. `validate_pending_predicates` — JIT-valida predicados complexos.

mod constant_fold;
mod constness;
mod ctx;
mod error;
mod fold;
mod jit;
mod predicates;
mod pureza;
mod result;
mod snapshot;
mod walk;

use std::collections::HashMap;

use kata_ast::Spanned;
use kata_core::EnumRegistry;
use kata_inference::{TypedExpr, TypedExprKind, TypedModule};

use constness::is_comptime_available;
use ctx::ModuleCtx;
use fold::fold_literal_calls;
use jit::jit_execute_expr;
use predicates::validate_pending_predicates;
use pureza::check_purity;
use result::result_to_literal;

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
/// Ver `module docs` para a arquitetura em 4 fases.
pub fn run_comptime_pass(
    typed: TypedModule,
    enum_registry: &EnumRegistry,
) -> Result<TypedModule, ComptimeError> {
    let mut current = typed;
    let mut snapshots: Vec<kata_core::snapshot::HeapSnapshotData> =
        std::mem::take(&mut current.snapshots);

    // Bindings comptime-available — construído incrementalmente.
    // Após avaliar uma constant, seu valor literal é adicionado aqui.
    // Uma constant posterior que referencia outra vê o binding no mapa.
    // O mapa também é injetado no mini TypedModule para o JIT resolver Idents
    // comptime-available.
    let mut comptime_bindings: HashMap<String, TypedExpr> = HashMap::new();

    // Clonar actions antes do loop para evitar conflito de borrow:
    // o ctx precisa de &actions (imutável) para jit_execute_expr, mas
    // precisamos mutar current.actions[i].body. Clonar resolve
    // (consistente com jit_execute_expr que já clona tudo).
    let actions_clone = current.actions.clone();
    let ctx = ModuleCtx {
        dispatch_table: &current.dispatch_table,
        type_env: &current.type_env,
        functions: &current.functions,
        actions: &actions_clone,
        struct_registry: &current.struct_registry,
        enum_registry,
    };

    // ── Fase 1: Avaliar constants (pre-pass linear, sem fixpoint) ──
    evaluate_constants(
        &mut current.constants,
        &ctx,
        &mut snapshots,
        &mut comptime_bindings,
    )?;

    // ── Fase 2: Fixpoint de fold_literal_calls (cascata de folds) ──
    // O resultado de um fold pode ser arg literal de outro fold.
    // Repete até que nenhuma mudança ocorra.
    loop {
        let mut changed = false;

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
        // Fold em constants: percorre o value de cada ConstantBinding.
        // Isto pega `constant result := f 41` onde f é named function
        // e 41 é literal — fold executa f(41) em compile-time.
        for binding in &mut current.constants {
            let (name, was_literal) = match &binding.node.kind {
                TypedExprKind::ConstantBinding { name, value } => {
                    (name.clone(), is_already_evaluated(&value.node))
                }
                _ => continue,
            };
            if let TypedExprKind::ConstantBinding { value, .. } = &mut binding.node.kind {
                fold_literal_calls(
                    &mut value.node,
                    &ctx,
                    &mut changed,
                    &mut snapshots,
                    &comptime_bindings,
                )?;
                // Se o fold transformou o value em literal, registrar no
                // comptime_bindings para que fold_constant_refs possa
                // substituir Ident(name) nos corpos de functions/actions.
                if !was_literal && is_already_evaluated(&value.node) {
                    comptime_bindings.insert(name, value.node.clone());
                }
            }
        }

        if !changed {
            break;
        }
    }

    current.snapshots = snapshots;

    // ── Fase 3: Substituir refs a constants nos corpos de functions e actions ──
    // Após o fixpoint, comptime_bindings contém os valores avaliados de todas
    // as constants. Functions e actions compilam em FunctionBuilders separados
    // que não têm acesso ao var_map do entry point (onde constants são
    // registradas). Esta passagem substitui Ident(name) pelo literal/snapshot
    // quando name é uma constant e não está mascarado por um binding local.
    constant_fold::fold_constant_refs_in_functions(&mut current.functions, &comptime_bindings)?;
    constant_fold::fold_constant_refs_in_actions(&mut current.actions, &comptime_bindings)?;

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

/// Pre-pass linear: avalia cada `ConstantBinding` uma vez, em ordem de
/// declaração. Sem fixpoint.
///
/// A ordem de declaração é a ordem de dependência — a inferência já garante
/// que forward references entre constants falham com `UnboundName`. Cada
/// constant avaliada é registrada em `comptime_bindings` imediatamente,
/// para que a próxima constant que a referencia a veja.
///
/// Constants importadas (já avaliadas pelo pipeline recursivo do módulo
/// exportador) são puladas — só precisam ser registradas em
/// `comptime_bindings` para `fold_constant_refs`.
fn evaluate_constants(
    constants: &mut [Spanned<TypedExpr>],
    ctx: &ModuleCtx<'_>,
    snapshots: &mut Vec<kata_core::snapshot::HeapSnapshotData>,
    comptime_bindings: &mut HashMap<String, TypedExpr>,
) -> Result<(), ComptimeError> {
    for binding in constants {
        let (name, value_span, value_clone) = match &binding.node.kind {
            TypedExprKind::ConstantBinding { name, value } => {
                (name.clone(), value.span, value.node.clone())
            }
            _ => continue,
        };

        // Pular constants já avaliadas (importadas de outro módulo, ou
        // já foldadas por iteração anterior). Registrar no mapa para que
        // fold_constant_refs possa substituir Ident(name) nos corpos.
        if is_already_evaluated(&value_clone) {
            comptime_bindings.insert(name, value_clone);
            continue;
        }

        // Pular Closures (chamadas de função) — o fold_literal_calls
        // cuida de Closures com args literais (ex: `f 41` onde f é
        // named function e 41 é literal). O passo de constants só
        // avalia expressões estruturais (ListLit, StructConstruct,
        // etc.), não chamadas de função.
        if matches!(value_clone.kind, TypedExprKind::Closure { .. }) {
            // Mas ainda precisamos registrar caso fold depois a avalie.
            // Usamos o value original — fold_literal_calls pode trocá-lo.
            comptime_bindings.insert(name, value_clone);
            continue;
        }

        // Validações de constness:
        // - ConstantLambda já detectado na inferência (C3).
        // - Pureza e comptime-availability continuam aqui (dependem do
        //   contexto de avaliação do comptime pass).
        if !is_comptime_available(&value_clone, &comptime_bindings) {
            return Err(ComptimeError::NotConsttime {
                reason: format!("constant {name} — expressão depende de valor runtime"),
            });
        }
        check_purity(&value_clone)?;

        let result = jit_execute_expr(&value_clone, ctx, comptime_bindings)?;
        let literal = result_to_literal(
            &result,
            &value_clone,
            snapshots,
            ctx.struct_registry,
            ctx.enum_registry,
        )?;
        // Substituir o value do ConstantBinding pelo literal.
        if let TypedExprKind::ConstantBinding { value, .. } = &mut binding.node.kind {
            **value = Spanned::new(literal.clone(), value_span);
        }
        comptime_bindings.insert(name, literal);
    }
    Ok(())
}

/// Verifica se o value de um ConstantBinding já foi avaliado (é literal
/// ou HeapSnapshot).
fn is_already_evaluated(expr: &TypedExpr) -> bool {
    matches!(
        &expr.kind,
        TypedExprKind::IntLit { .. }
            | TypedExprKind::FloatLit { .. }
            | TypedExprKind::TextLit { .. }
            | TypedExprKind::Unit
            | TypedExprKind::HeapSnapshot { .. }
            | TypedExprKind::VariantQual { .. }
    )
}