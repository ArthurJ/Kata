//! Pass pós-inferência: marca `ChannelCreate` como `cross_process: true`
//! quando o canal flui para `spawn!`, e resolve `Var("T0")` no tipo do
//! canal para o tipo concreto inferido do uso.
//!
//! O codegen usa `cross_process` para decidir entre `kata_rt_channel_create`
//! (in-process, Mutex/Condvar) e `kata_rt_ipc_channel_create` (cross-process,
//! pipe Unix + serialização).
//!
//! A resolução de `Var("T0")` é necessária porque `channel!()` cria
//! `Ty::Var("T0")` como tipo de elemento, e o `type_compatible` em `csp.rs`
//! aceita `Var` como coringa sem unificar. O `fork!` e `spawn!` fazem
//! `env.apply_substitutions` durante a inferência (resolve bindings do
//! TypeEnv), mas a TAST já foi construída com `Var("T0")`. Este pass aplica
//! as substituições na TAST também, para que o codegen veja o tipo concreto
//! no `ChannelCreate` (necessário para `lookup_type_id` em canais IPC).
//!
//! ## Algoritmo
//!
//! 1. Coleta mapeamentos `var_name → span` de todos os `Let`/`Var`/`LetDestruct`
//!    onde o valor é um `ChannelCreate`, rastreando também acessos via
//!    `IndexAccess` (`let tx := ch.0` — o typeck lowered `.0` para IndexAccess
//!    compile-time em tuplas). Itera até fixpoint para rastrear cadeias de
//!    bindings.
//! 2. Percorre a TAST procurando `Spawn` cujos args contêm `Ident` com nome
//!    no mapeamento. Para cada match, coleta o span do `ChannelCreate`.
//! 3. Marca os `ChannelCreate` correspondentes em todo o módulo (pre_entry,
//!    entry, e Actions).
//! 4. Coleta substituições `Var(name) → tipo_concreto` a partir de
//!    `ChannelSend` onde o tipo do value é concreto e o tipo do channel é
//!    `Sender(Var(name))`, e de `ChannelRecv` onde o tipo do channel é
//!    `Receiver(Var(name))` e o `recv_ty` é concreto.
//! 5. Aplica as substituições em todos os `Ty` da TAST.
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

use crate::typed::{TypedExpr, TypedExprKind, TypedModule};
use kata_core::ty::Ty;

use super::walk;

/// Rastreia canais que fluem para `spawn!` e marca `ChannelCreate` como
/// `cross_process: true`. Também resolve `Var("T0")` na TAST para o tipo
/// concreto inferido do uso.
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
        collect_spawn_spans(
            &typed_module.entry.node,
            &channel_bindings,
            &mut spans_to_mark,
        );

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

    // ── Resolução de Var("T0") na TAST ──
    // O spawn!/fork! aplica env.apply_substitutions durante a inferência,
    // que resolve Var nos bindings do TypeEnv. Mas o ChannelCreate já foi
    // construído antes do spawn! e ainda tem Var no elem_ty e no expr.ty.
    // ── Resolução de Var("T0") na TAST ──
    // O spawn!/fork! aplica env.apply_substitutions durante a inferência,
    // que resolve Var nos bindings do TypeEnv. Mas o ChannelCreate já foi
    // construído antes do spawn! e ainda tem Var no elem_ty e no expr.ty.
    // Coletamos os tipos concretos dos ChannelSend (onde o tipo já foi
    // resolvido pelo spawn!) e mutamos os ChannelCreate correspondentes
    // diretamente pelo span.
    if !channel_bindings.is_empty() {
        // Mapa: span_do_ChannelCreate -> tipo_concreto_do_elemento
        let mut create_types: HashMap<kata_ast::Span, Ty> = HashMap::new();

        for expr in &typed_module.pre_entry {
            collect_concrete_channel_types(&expr.node, &channel_bindings, &mut create_types);
        }
        collect_concrete_channel_types(
            &typed_module.entry.node,
            &channel_bindings,
            &mut create_types,
        );

        if !create_types.is_empty() {
            for expr in &mut typed_module.pre_entry {
                resolve_channel_create(&mut expr.node, &create_types);
            }
            resolve_channel_create(&mut typed_module.entry.node, &create_types);
        }
    }
}

/// Coleta bindings de `ChannelCreate` de uma expressão TAST e os adiciona
/// ao mapa `channel_bindings`. Rastreia:
/// - `let ch := channel!()` → `ch → span`
/// - `let (tx, rx) := channel!()` (LetDestruct) → `tx, rx → span`
/// - `let tx := ch.0` (IndexAccess de binding de canal) → `tx → span`
/// - `var ch := channel!()` → `ch → span`
///
/// Nota: `ch.0` em tupla é lowered pelo typeck para `IndexAccess` (não
/// `FieldAccess`), porque tuplas usam indexação compile-time com bounds check.
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
            // O typeck lowered `.0` para IndexAccess em tuplas.
            if let TypedExprKind::IndexAccess { expr: inner, .. } = &value.node.kind {
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

/// Coleta tipos concretos de canais a partir de `ChannelSend` onde o
/// `channel` é um `Ident` cujo nome está em `channel_bindings`. Extrai
/// o tipo do elemento do `Sender(inner)` e mapeia `span_do_ChannelCreate
/// → inner`.
///
/// Também coleta de `ChannelRecv` onde o `channel` é `Ident` em
/// `channel_bindings` e o `recv_ty` é concreto.
fn collect_concrete_channel_types(
    expr: &TypedExpr,
    channel_bindings: &HashMap<String, kata_ast::Span>,
    create_types: &mut HashMap<kata_ast::Span, Ty>,
) {
    walk::for_each_subexpr(expr, &mut |e| {
        match &e.kind {
            TypedExprKind::ChannelSend { channel, .. } => {
                if let TypedExprKind::Ident { name } = &channel.node.kind {
                    if let Some(span) = channel_bindings.get(name) {
                        if let Ty::Sender(inner) = &channel.node.ty {
                            if !matches!(inner.as_ref(), Ty::Var(_)) {
                                create_types
                                    .entry(*span)
                                    .or_insert_with(|| inner.as_ref().clone());
                            }
                        }
                    }
                }
            }
            TypedExprKind::ChannelRecv {
                channel, recv_ty, ..
            } => {
                if let TypedExprKind::Ident { name } = &channel.node.kind {
                    if let Some(span) = channel_bindings.get(name) {
                        if !matches!(recv_ty, Ty::Var(_)) {
                            create_types.entry(*span).or_insert_with(|| recv_ty.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        true
    });
}

/// Muta `ChannelCreate` na TAST cujo span está em `create_types`,
/// substituindo `elem_ty` e `expr.ty` pelo tipo concreto.
fn resolve_channel_create(expr: &mut TypedExpr, create_types: &HashMap<kata_ast::Span, Ty>) {
    walk::for_each_subexpr_mut(expr, &mut |e| {
        if let TypedExprKind::ChannelCreate { elem_ty, kind, .. } = &mut e.kind {
            if let Some(concrete_ty) = create_types.get(&e.span) {
                *elem_ty = concrete_ty.clone();
                // Atualiza expr.ty para Tuple([Sender(concrete), Receiver(concrete)])
                // ou Tuple([Sender(concrete), ReceiverFactory(concrete)]) para broadcast.
                e.ty = match kind {
                    crate::typed::ChannelKind::Broadcast => Ty::Tuple(vec![
                        Ty::Sender(Box::new(concrete_ty.clone())),
                        Ty::ReceiverFactory(Box::new(concrete_ty.clone())),
                    ]),
                    _ => Ty::Tuple(vec![
                        Ty::Sender(Box::new(concrete_ty.clone())),
                        Ty::Receiver(Box::new(concrete_ty.clone())),
                    ]),
                };
                return false; // não desce mais
            }
        }
        true
    });
}
