//! Pass pós-inferência: marca `ChannelCreate` como `cross_process: true`
//! quando o canal flui para `spawn!`.
//!
//! O codegen usa `cross_process` para decidir entre `kata_rt_channel_create`
//! (in-process, Mutex/Condvar) e `kata_rt_ipc_channel_create` (cross-process,
//! pipe Unix + serialização).
//!
//! ## Algoritmo
//!
//! 1. Coleta mapeamentos `var_name → span` de todos os `Let`/`Var`/`LetDestruct`
//!    onde o valor é um `ChannelCreate`, rastreando também acessos via
//!    `FieldAccess` (`let tx := ch.0`). Itera até fixpoint para rastrear
//!    cadeias de bindings.
//! 2. Percorre a TAST procurando `Spawn` cujos args contêm `Ident` com nome
//!    no mapeamento. Para cada match, coleta o span do `ChannelCreate`.
//! 3. Marca os `ChannelCreate` correspondentes em todo o módulo (pre_entry,
//!    entry, e Actions).
//!
//! ## Escopo
//!
//! Para o entry point, os bindings são coletados de **todo o pre_entry + entry**
//! juntos, porque os `let` podem estar no pre_entry e o `spawn!` no entry.
//! A marcação também é global sobre pre_entry + entry, porque o `ChannelCreate`
//! pode estar numa expr e o `Spawn` em outra. Para Actions, cada action tem
//! seu próprio escopo.
//!
//! ## Limitações
//!
//! - Não rastreia canais passados por parâmetro de Action (apenas `let`
//!   bindings diretos).
//! - Não rastreia canais em tuplas/dicts aninhadas nos args do `Spawn`
//!   (apenas `Ident` direto e tupla plana de `Ident`s).
//! - Over-approximation possível se o mesmo nome é reusado em escopos
//!   diferentes — aceitável para v1.

use std::collections::HashMap;

use kata_ast::Spanned;
use crate::typed::{TypedExpr, TypedExprKind, TypedModule};

use super::walk;

/// Rastreia canais que fluem para `spawn!` e marca `ChannelCreate` como
/// `cross_process: true`.
pub(crate) fn run(typed_module: &mut TypedModule) {
    // ── Entry point: pre_entry + entry ──
    // Coleta bindings de ChannelCreate de pre_entry + entry juntos.
    let mut channel_bindings: HashMap<String, kata_ast::Span> = HashMap::new();
    loop {
        let prev_len = channel_bindings.len();
        for expr in &typed_module.pre_entry {
            collect_channel_bindings(&expr.node, &mut channel_bindings);
        }
        collect_channel_bindings(&typed_module.entry.node, &mut channel_bindings);
        if channel_bindings.len() == prev_len {
            break;
        }
    }

    if !channel_bindings.is_empty() {
        // Coleta spans de ChannelCreate referenciados em Spawn args.
        let mut spans_to_mark: Vec<kata_ast::Span> = Vec::new();
        for expr in &typed_module.pre_entry {
            collect_spawn_spans(&expr.node, &channel_bindings, &mut spans_to_mark);
        }
        collect_spawn_spans(&typed_module.entry.node, &channel_bindings, &mut spans_to_mark);

        // Marca os ChannelCreate correspondentes em todo o módulo (pre_entry + entry).
        for expr in &mut typed_module.pre_entry {
            for span in &spans_to_mark {
                mark_channel_create_by_span(&mut expr.node, *span);
            }
        }
        for span in &spans_to_mark {
            mark_channel_create_by_span(&mut typed_module.entry.node, *span);
        }
    }

    // ── Actions: cada action tem seu próprio escopo ──
    for action in &mut typed_module.actions {
        let mut action_bindings: HashMap<String, kata_ast::Span> = HashMap::new();
        loop {
            let prev_len = action_bindings.len();
            for stmt in &action.body {
                collect_channel_bindings(&stmt.node, &mut action_bindings);
            }
            if action_bindings.len() == prev_len {
                break;
            }
        }
        if !action_bindings.is_empty() {
            let mut spans_to_mark: Vec<kata_ast::Span> = Vec::new();
            for stmt in &action.body {
                collect_spawn_spans(&stmt.node, &action_bindings, &mut spans_to_mark);
            }
            for stmt in &mut action.body {
                for span in &spans_to_mark {
                    mark_channel_create_by_span(&mut stmt.node, *span);
                }
            }
        }
    }
}

/// Coleta bindings de `ChannelCreate` de uma expressão TAST e os adiciona
/// ao mapa `channel_bindings`. Rastreia:
/// - `let ch := channel!()` → `ch → span`
/// - `let (tx, rx) := channel!()` (LetDestruct) → `tx, rx → span`
/// - `let tx := ch.0` (FieldAccess de binding de canal) → `tx → span`
/// - `var ch := channel!()` → `ch → span`
fn collect_channel_bindings(
    expr: &TypedExpr,
    channel_bindings: &mut HashMap<String, kata_ast::Span>,
) {
    walk::for_each_subexpr(expr, &mut |e| {
        // let ch := channel!()
        if let TypedExprKind::Let { name, value, .. } = &e.kind {
            if matches!(value.node.kind, TypedExprKind::ChannelCreate { .. }) {
                channel_bindings.insert(name.clone(), value.span);
            }
            // let tx := ch.0  (ch já está no mapa)
            if let TypedExprKind::FieldAccess { expr: inner, .. } = &value.node.kind {
                if let TypedExprKind::Ident { name: inner_name } = &inner.node.kind {
                    if let Some(span) = channel_bindings.get(inner_name) {
                        channel_bindings.insert(name.clone(), *span);
                    }
                }
            }
        }
        // let (tx, rx) := channel!()
        if let TypedExprKind::LetDestruct {
            temp_name,
            value,
            bindings,
        } = &e.kind
        {
            if matches!(value.node.kind, TypedExprKind::ChannelCreate { .. }) {
                channel_bindings.insert(temp_name.clone(), value.span);
                for (name, _) in bindings {
                    channel_bindings.insert(name.clone(), value.span);
                }
            }
        }
        // var ch := channel!()
        if let TypedExprKind::Var { name, value, .. } = &e.kind {
            if matches!(value.node.kind, TypedExprKind::ChannelCreate { .. }) {
                channel_bindings.insert(name.clone(), value.span);
            }
        }
        true // continuar descida
    });
}

/// Percorre a expressão procurando `Spawn` cujos args contêm `Ident` com
/// nome no mapeamento `channel_bindings`. Coleta os spans dos
/// `ChannelCreate` correspondentes.
fn collect_spawn_spans(
    expr: &TypedExpr,
    channel_bindings: &HashMap<String, kata_ast::Span>,
    spans: &mut Vec<kata_ast::Span>,
) {
    walk::for_each_subexpr(expr, &mut |e| {
        if let TypedExprKind::Spawn { args, .. } = &e.kind {
            collect_channel_spans(&args.node, channel_bindings, spans);
        }
        true // continuar descida
    });
}

/// Recursivamente coleta spans de `ChannelCreate` referenciados por `Ident`
/// nos args do `Spawn`.
fn collect_channel_spans(
    expr: &TypedExpr,
    bindings: &HashMap<String, kata_ast::Span>,
    spans: &mut Vec<kata_ast::Span>,
) {
    walk::for_each_subexpr(expr, &mut |e| {
        if let TypedExprKind::Ident { name } = &e.kind {
            if let Some(span) = bindings.get(name) {
                spans.push(*span);
                return false; // não desce (Ident não tem filhos)
            }
        }
        true
    });
}

/// Marca um `ChannelCreate` na TAST (por span) como `cross_process: true`.
fn mark_channel_create_by_span(expr: &mut TypedExpr, target_span: kata_ast::Span) {
    walk::for_each_subexpr_mut(expr, &mut |e| {
        if let TypedExprKind::ChannelCreate { cross_process, .. } = &mut e.kind {
            if e.span == target_span {
                *cross_process = true;
                return false; // não desce mais
            }
        }
        true
    });
}