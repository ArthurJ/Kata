//! Proibição de recursão em Actions.
//!
//! Actions executam em fibers com stack fixa (1 MB). Recursão — direta ou
//! indireta — poderia estourar a stack sem o programador perceber. Esta
//! análise constrói o call graph das Actions e rejeita ciclos via DFS.
//!
//! Só Actions do usuário (`ffi_symbol: None`) participam do call graph.
//! Builtins (`echo!`, `panic!`, `assert!`) têm `ffi_symbol: Some(...)` e
//! são ignorados — não são Actions recursivas.

use std::collections::HashMap;

use kata_core::ty::Ty;
use kata_diagnostics::MiddleError;

use crate::typed::{
    FusedStage, TypedAction, TypedExpr, TypedExprKind, TypedLambdaClause, TypedMatchArm,
    TypedPattern,
};

/// Verifica que nenhuma Action é recursiva (direta ou indireta).
///
/// Constrói o call graph: para cada Action, coleta os nomes das Actions
/// que ela chama (percorrendo recursivamente toda a TAST do body). Depois
/// roda DFS com coloring (white/gray/black) para detectar ciclos.
///
/// Retorna `RecursiveAction` no primeiro ciclo encontrado, com a chain
/// (ex: "A → B → A") e o span do `ActionCall` que fecha o ciclo.
pub(crate) fn check_action_recursion(actions: &[TypedAction]) -> Result<(), MiddleError> {
    let mut call_graph = build_call_graph(actions);
    collect_indirect_edges(actions, &mut call_graph);
    detect_cycle(&call_graph)
}

/// Constrói o call graph: `Action name → Vec<callee names>`.
///
/// Só inclui `ActionCall` com `ffi_symbol: None` (Actions do usuário).
/// Builtins são ignorados.
fn build_call_graph(actions: &[TypedAction]) -> HashMap<String, Vec<(String, kata_ast::Span)>> {
    let mut graph: HashMap<String, Vec<(String, kata_ast::Span)>> = HashMap::new();
    for action in actions {
        let mut callees = Vec::new();
        for stmt in &action.body {
            collect_action_calls(&stmt.node, &stmt.span, &mut callees);
        }
        graph.insert(action.name.clone(), callees);
    }
    graph
}

/// Segunda passada do call graph: propaga def-use interprocedural para
/// invocação indireta via parâmetro first-class (PRD §4.3, 1 nível).
///
/// Quando `dispatcher!(worker_a, 42)` passa `worker_a` como param
/// `job :: Action(Int) => Unit`, e `dispatcher` chama `job!(payload)`,
/// o call graph precisa da aresta `dispatcher → worker_a`. Se
/// `worker_a` chama `dispatcher` (direta ou indiretamente), há ciclo.
///
/// Algoritmo (1 nível apenas — cadeias de intermediárias ficam para depois):
/// 1. Para cada action A, encontra "callable params": params com
///    `ty: Ty::Action(..)`.
/// 2. Verifica se algum callable param é invocado dentro de A (aparece como
///    `callee` em `ActionCall` com `indirect_callee: Some(..)`).
/// 3. Se sim, procura todos os call sites de A nas outras actions (via
///    `ActionCall { callee: "A" }` ou `Fork { action_name: "A" }`).
/// 4. Em cada call site, verifica se algum arg é `Ident { name }` onde name
///    é uma Action definida pelo usuário.
/// 5. Para cada arg desse tipo, registra aresta `A → arg_name`.
fn collect_indirect_edges(
    actions: &[TypedAction],
    graph: &mut HashMap<String, Vec<(String, kata_ast::Span)>>,
) {
    // Conjunto dos nomes de Actions definidas pelo usuário — usado para
    // distinguir first-class Action reference de variável local comum.
    let action_names: std::collections::HashSet<&str> =
        actions.iter().map(|a| a.name.as_str()).collect();

    for target in actions {
        // 1. Params com ty: Ty::Action(..) — callable params.
        let callable_params: Vec<&str> = target
            .param_types
            .iter()
            .zip(target.param_names.iter())
            .filter_map(|(ty, name_opt)| {
                if matches!(ty, Ty::Action(..)) {
                    name_opt.as_deref()
                } else {
                    None
                }
            })
            .collect();
        if callable_params.is_empty() {
            continue;
        }

        // 2. Quais callable params são invocados? Procura por
        //    ActionCall { callee, indirect_callee: Some(..), ffi_symbol: None }
        //    onde callee matches a callable param.
        let mut invoked_params: Vec<String> = Vec::new();
        for stmt in &target.body {
            collect_invoked_callable_params(&stmt.node, &callable_params, &mut invoked_params);
        }
        if invoked_params.is_empty() {
            continue;
        }

        // 3. Encontra call sites de target nas outras actions (e em si
        //    mesma — recursão indireta via param é possível).
        for caller in actions {
            for stmt in &caller.body {
                let mut edges = Vec::new();
                find_call_sites_of(
                    &stmt.node,
                    &stmt.span,
                    &target.name,
                    &action_names,
                    &mut edges,
                );
                // edges: Vec<(span, Vec<arg_action_name>)>
                for (span, arg_action_names) in edges {
                    for arg_name in arg_action_names {
                        // 5. Registra aresta target → arg_name.
                        graph
                            .entry(target.name.clone())
                            .or_default()
                            .push((arg_name, span));
                    }
                }
            }
        }
    }
}

/// Procura por ActionCall com `indirect_callee: Some(..)` cujo `callee`
/// matches um dos nomes em `callable_params`. Adiciona o nome do param
/// invocado em `out` (sem duplicar — usa verificação simples).
fn collect_invoked_callable_params(
    expr: &TypedExpr,
    callable_params: &[&str],
    out: &mut Vec<String>,
) {
    match &expr.kind {
        TypedExprKind::ActionCall {
            callee,
            indirect_callee: Some(_),
            ffi_symbol: None,
            ..
        } => {
            if callable_params.contains(&callee.as_str())
                && !out.iter().any(|p| p == callee)
            {
                out.push(callee.clone());
            }
        }
        _ => {}
    }
    // Recursão nos sub-nós para encontrar mais invocações.
    traverse_for_invoked_params(expr, callable_params, out);
}

/// Traversal genérico para achar ActionCall indirect-callee nos sub-nós.
fn traverse_for_invoked_params(
    expr: &TypedExpr,
    callable_params: &[&str],
    out: &mut Vec<String>,
) {
    match &expr.kind {
        TypedExprKind::ActionCall {
            args,
            indirect_callee,
            ffi_symbol,
            ..
        } => {
            collect_action_calls_recursive_for_invoked(&args.node, callable_params, out);
            if let Some(ic) = indirect_callee {
                collect_invoked_callable_params(&ic.node, callable_params, out);
            }
            // ffi_symbol Some: args ainda podem conter indirect calls.
            if ffi_symbol.is_some() {
                collect_action_calls_recursive_for_invoked(&args.node, callable_params, out);
            }
        }
        TypedExprKind::Closure { callee, args, .. } => {
            traverse_for_invoked_params(&callee.node, callable_params, out);
            for arg in args {
                traverse_for_invoked_params(&arg.node, callable_params, out);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => {
            traverse_for_invoked_params(&expr.node, callable_params, out);
        }
        TypedExprKind::Grouping { inner } => {
            traverse_for_invoked_params(&inner.node, callable_params, out);
        }
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                traverse_for_invoked_params(&el.node, callable_params, out);
            }
        }
        TypedExprKind::Let { value, .. }
        | TypedExprKind::LetDestruct { value, .. }
        | TypedExprKind::Var { value, .. } => {
            traverse_for_invoked_params(&value.node, callable_params, out);
        }
        TypedExprKind::Reassign { value, .. } => {
            traverse_for_invoked_params(&value.node, callable_params, out);
        }
        TypedExprKind::Return(inner) => {
            traverse_for_invoked_params(&inner.node, callable_params, out);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            traverse_for_invoked_params(&scrutinee.node, callable_params, out);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    traverse_for_invoked_params(&guard.node, callable_params, out);
                }
                traverse_for_invoked_params(&arm.body.node, callable_params, out);
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses {
                for guard in &clause.guards {
                    if let Some(cond) = &guard.condition {
                        traverse_for_invoked_params(&cond.node, callable_params, out);
                    }
                    traverse_for_invoked_params(&guard.body.node, callable_params, out);
                }
                for wb in &clause.with_bindings {
                    traverse_for_invoked_params(&wb.value.node, callable_params, out);
                }
                if clause.guards.is_empty() {
                    traverse_for_invoked_params(&clause.body.node, callable_params, out);
                }
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                traverse_for_invoked_params(&stmt.node, callable_params, out);
            }
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for val in values {
                traverse_for_invoked_params(&val.node, callable_params, out);
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => {
            traverse_for_invoked_params(&expr.node, callable_params, out);
        }
        TypedExprKind::IndexAccess { expr, .. } => {
            traverse_for_invoked_params(&expr.node, callable_params, out);
        }
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                traverse_for_invoked_params(&el.node, callable_params, out);
            }
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (key, val) in entries {
                traverse_for_invoked_params(&key.node, callable_params, out);
                traverse_for_invoked_params(&val.node, callable_params, out);
            }
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements {
                traverse_for_invoked_params(&el.node, callable_params, out);
            }
        }
        TypedExprKind::RangeLit { start, step, end, .. } => {
            traverse_for_invoked_params(&start.node, callable_params, out);
            traverse_for_invoked_params(&step.node, callable_params, out);
            traverse_for_invoked_params(&end.node, callable_params, out);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            traverse_for_invoked_params(&iterable.node, callable_params, out);
            for stmt in body {
                traverse_for_invoked_params(&stmt.node, callable_params, out);
            }
        }
        TypedExprKind::In { item, collection } => {
            traverse_for_invoked_params(&item.node, callable_params, out);
            traverse_for_invoked_params(&collection.node, callable_params, out);
        }
        TypedExprKind::Map { callback, collection, .. }
        | TypedExprKind::Filter { callback, collection, .. } => {
            traverse_for_invoked_params(&callback.node, callable_params, out);
            traverse_for_invoked_params(&collection.node, callable_params, out);
        }
        TypedExprKind::Fold { callback, initial, collection, .. } => {
            traverse_for_invoked_params(&callback.node, callable_params, out);
            traverse_for_invoked_params(&initial.node, callable_params, out);
            traverse_for_invoked_params(&collection.node, callable_params, out);
        }
        TypedExprKind::FusedStream { stages, source, .. } => {
            traverse_for_invoked_params(&source.node, callable_params, out);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                traverse_for_invoked_params(&cb.node, callable_params, out);
            }
        }
        TypedExprKind::ChannelSend { channel, value } => {
            traverse_for_invoked_params(&channel.node, callable_params, out);
            traverse_for_invoked_params(&value.node, callable_params, out);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            traverse_for_invoked_params(&channel.node, callable_params, out);
        }
        TypedExprKind::Select { arms, timeout_ms, timeout_body, .. } => {
            for arm in arms {
                traverse_for_invoked_params(&arm.channel.node, callable_params, out);
                traverse_for_invoked_params(&arm.body.node, callable_params, out);
            }
            if let Some(tm) = timeout_ms {
                traverse_for_invoked_params(&tm.node, callable_params, out);
            }
            if let Some(tb) = timeout_body {
                traverse_for_invoked_params(&tb.node, callable_params, out);
            }
        }
        TypedExprKind::Fork { args, .. } => {
            traverse_for_invoked_params(&args.node, callable_params, out);
        }
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            traverse_for_invoked_params(&factory.node, callable_params, out);
        }
        TypedExprKind::ChannelCreate { .. }
        | TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::VariantConstruct { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
    }
}

/// Helper: chama `collect_invoked_callable_params` recursivamente em
/// todos sub-nós. Mesma lógica de `traverse_for_invoked_params` mas é
/// usado para args dentro de ActionCall (que já desceu um nível).
fn collect_action_calls_recursive_for_invoked(
    expr: &TypedExpr,
    callable_params: &[&str],
    out: &mut Vec<String>,
) {
    traverse_for_invoked_params(expr, callable_params, out);
}

/// Encontra call sites de `target_name` na expressão. Para cada call site,
/// retorna `(span, Vec<arg_action_name>)` — a lista de args que são
/// first-class Action references (Ident com nome de Action definida).
fn find_call_sites_of(
    expr: &TypedExpr,
    span: &kata_ast::Span,
    target_name: &str,
    action_names: &std::collections::HashSet<&str>,
    out: &mut Vec<(kata_ast::Span, Vec<String>)>,
) {
    match &expr.kind {
        TypedExprKind::ActionCall {
            callee,
            args,
            ffi_symbol: None,
            ..
        } if callee == target_name => {
            let arg_actions = extract_action_idents_from_args(&args.node, action_names);
            if !arg_actions.is_empty() {
                out.push((*span, arg_actions));
            }
        }
        TypedExprKind::Fork { action_name, args, .. } if action_name == target_name => {
            let arg_actions = extract_action_idents_from_args(&args.node, action_names);
            if !arg_actions.is_empty() {
                out.push((expr.span, arg_actions));
            }
        }
        _ => {}
    }
    // Recursão nos sub-nós.
    traverse_for_call_sites(expr, target_name, action_names, out);
}

/// Traversal genérico para achar call sites em sub-nós.
fn traverse_for_call_sites(
    expr: &TypedExpr,
    target_name: &str,
    action_names: &std::collections::HashSet<&str>,
    out: &mut Vec<(kata_ast::Span, Vec<String>)>,
) {
    match &expr.kind {
        TypedExprKind::ActionCall {
            callee,
            args,
            indirect_callee,
            ffi_symbol,
            ..
        } => {
            // Se é um call site de target_name (não match acima — pode ter
            // ffi_symbol Some), ainda verificamos args.
            if callee != target_name || ffi_symbol.is_some() {
                traverse_for_call_sites(&args.node, target_name, action_names, out);
            }
            if let Some(ic) = indirect_callee {
                traverse_for_call_sites(&ic.node, target_name, action_names, out);
            }
            if ffi_symbol.is_some() {
                traverse_for_call_sites(&args.node, target_name, action_names, out);
            }
        }
        TypedExprKind::Fork { args, .. } => {
            traverse_for_call_sites(&args.node, target_name, action_names, out);
        }
        TypedExprKind::Closure { callee, args, .. } => {
            traverse_for_call_sites(&callee.node, target_name, action_names, out);
            for arg in args {
                traverse_for_call_sites(&arg.node, target_name, action_names, out);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => {
            traverse_for_call_sites(&expr.node, target_name, action_names, out);
        }
        TypedExprKind::Grouping { inner } => {
            traverse_for_call_sites(&inner.node, target_name, action_names, out);
        }
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                traverse_for_call_sites(&el.node, target_name, action_names, out);
            }
        }
        TypedExprKind::Let { value, .. }
        | TypedExprKind::LetDestruct { value, .. }
        | TypedExprKind::Var { value, .. } => {
            traverse_for_call_sites(&value.node, target_name, action_names, out);
        }
        TypedExprKind::Reassign { value, .. } => {
            traverse_for_call_sites(&value.node, target_name, action_names, out);
        }
        TypedExprKind::Return(inner) => {
            traverse_for_call_sites(&inner.node, target_name, action_names, out);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            traverse_for_call_sites(&scrutinee.node, target_name, action_names, out);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    traverse_for_call_sites(&guard.node, target_name, action_names, out);
                }
                traverse_for_call_sites(&arm.body.node, target_name, action_names, out);
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses {
                for guard in &clause.guards {
                    if let Some(cond) = &guard.condition {
                        traverse_for_call_sites(&cond.node, target_name, action_names, out);
                    }
                    traverse_for_call_sites(&guard.body.node, target_name, action_names, out);
                }
                for wb in &clause.with_bindings {
                    traverse_for_call_sites(&wb.value.node, target_name, action_names, out);
                }
                if clause.guards.is_empty() {
                    traverse_for_call_sites(&clause.body.node, target_name, action_names, out);
                }
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                traverse_for_call_sites(&stmt.node, target_name, action_names, out);
            }
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for val in values {
                traverse_for_call_sites(&val.node, target_name, action_names, out);
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => {
            traverse_for_call_sites(&expr.node, target_name, action_names, out);
        }
        TypedExprKind::IndexAccess { expr, .. } => {
            traverse_for_call_sites(&expr.node, target_name, action_names, out);
        }
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                traverse_for_call_sites(&el.node, target_name, action_names, out);
            }
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (key, val) in entries {
                traverse_for_call_sites(&key.node, target_name, action_names, out);
                traverse_for_call_sites(&val.node, target_name, action_names, out);
            }
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements {
                traverse_for_call_sites(&el.node, target_name, action_names, out);
            }
        }
        TypedExprKind::RangeLit { start, step, end, .. } => {
            traverse_for_call_sites(&start.node, target_name, action_names, out);
            traverse_for_call_sites(&step.node, target_name, action_names, out);
            traverse_for_call_sites(&end.node, target_name, action_names, out);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            traverse_for_call_sites(&iterable.node, target_name, action_names, out);
            for stmt in body {
                traverse_for_call_sites(&stmt.node, target_name, action_names, out);
            }
        }
        TypedExprKind::In { item, collection } => {
            traverse_for_call_sites(&item.node, target_name, action_names, out);
            traverse_for_call_sites(&collection.node, target_name, action_names, out);
        }
        TypedExprKind::Map { callback, collection, .. }
        | TypedExprKind::Filter { callback, collection, .. } => {
            traverse_for_call_sites(&callback.node, target_name, action_names, out);
            traverse_for_call_sites(&collection.node, target_name, action_names, out);
        }
        TypedExprKind::Fold { callback, initial, collection, .. } => {
            traverse_for_call_sites(&callback.node, target_name, action_names, out);
            traverse_for_call_sites(&initial.node, target_name, action_names, out);
            traverse_for_call_sites(&collection.node, target_name, action_names, out);
        }
        TypedExprKind::FusedStream { stages, source, .. } => {
            traverse_for_call_sites(&source.node, target_name, action_names, out);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                traverse_for_call_sites(&cb.node, target_name, action_names, out);
            }
        }
        TypedExprKind::ChannelSend { channel, value } => {
            traverse_for_call_sites(&channel.node, target_name, action_names, out);
            traverse_for_call_sites(&value.node, target_name, action_names, out);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            traverse_for_call_sites(&channel.node, target_name, action_names, out);
        }
        TypedExprKind::Select { arms, timeout_ms, timeout_body, .. } => {
            for arm in arms {
                traverse_for_call_sites(&arm.channel.node, target_name, action_names, out);
                traverse_for_call_sites(&arm.body.node, target_name, action_names, out);
            }
            if let Some(tm) = timeout_ms {
                traverse_for_call_sites(&tm.node, target_name, action_names, out);
            }
            if let Some(tb) = timeout_body {
                traverse_for_call_sites(&tb.node, target_name, action_names, out);
            }
        }
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            traverse_for_call_sites(&factory.node, target_name, action_names, out);
        }
        TypedExprKind::ChannelCreate { .. }
        | TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::VariantConstruct { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
    }
}

/// Extrai Ident names que são Actions definidas pelo usuário da tupla
/// de args de um call site. Aparecem como elementos de Tuple (calls
/// com mais de 1 arg) ou como a própria expr (call com 1 arg).
fn extract_action_idents_from_args(
    args_expr: &TypedExpr,
    action_names: &std::collections::HashSet<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    match &args_expr.kind {
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                extract_action_idents(&el.node, action_names, &mut out);
            }
        }
        TypedExprKind::Unit => {}
        _ => {
            extract_action_idents(args_expr, action_names, &mut out);
        }
    }
    out
}

/// Verifica se expr é Ident { name } onde name é Action definida.
fn extract_action_idents(
    expr: &TypedExpr,
    action_names: &std::collections::HashSet<&str>,
    out: &mut Vec<String>,
) {
    if let TypedExprKind::Ident { name } = &expr.kind {
        if action_names.contains(name.as_str()) {
            out.push(name.clone());
        }
    }
}

/// Coleta recursivamente todos os `ActionCall` com `ffi_symbol: None`
/// na árvore da TAST. `ActionCall` pode estar aninhado em Match arms,
/// Let/Var values, Return, Tuple elements, Grouping, etc.
fn collect_action_calls(
    expr: &TypedExpr,
    span: &kata_ast::Span,
    out: &mut Vec<(String, kata_ast::Span)>,
) {
    match &expr.kind {
        TypedExprKind::ActionCall {
            callee,
            ffi_symbol: None,
            ..
        } => {
            out.push((callee.clone(), *span));
        }
        TypedExprKind::ActionCall {
            args,
            ffi_symbol: Some(_),
            ..
        } => {
            // Builtin — não entra no call graph, mas args podem conter ActionCalls.
            collect_action_calls(&args.node, &args.span, out);
        }
        TypedExprKind::Closure { callee, args, .. } => {
            collect_action_calls(&callee.node, &callee.span, out);
            for arg in args {
                collect_action_calls(&arg.node, &arg.span, out);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => {
            collect_action_calls(&expr.node, &expr.span, out);
        }
        TypedExprKind::Grouping { inner } => {
            collect_action_calls(&inner.node, &inner.span, out);
        }
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                collect_action_calls(&el.node, &el.span, out);
            }
        }
        TypedExprKind::Let { value, .. }
        | TypedExprKind::LetDestruct { value, .. }
        | TypedExprKind::Var { value, .. } => {
            collect_action_calls(&value.node, &value.span, out);
        }
        TypedExprKind::Reassign { value, .. } => {
            collect_action_calls(&value.node, &value.span, out);
        }
        TypedExprKind::Return(inner) => {
            collect_action_calls(&inner.node, &inner.span, out);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            collect_action_calls(&scrutinee.node, &scrutinee.span, out);
            for arm in arms {
                collect_match_arm_calls(arm, out);
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses {
                collect_lambda_clause_calls(clause, out);
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                collect_action_calls(&stmt.node, &stmt.span, out);
            }
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for val in values {
                collect_action_calls(&val.node, &val.span, out);
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => {
            collect_action_calls(&expr.node, &expr.span, out);
        }
        TypedExprKind::IndexAccess { expr, .. } => {
            collect_action_calls(&expr.node, &expr.span, out);
        }
        // ── Coleções — recursão nos elementos ──
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                collect_action_calls(&el.node, &el.span, out);
            }
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (key, val) in entries {
                collect_action_calls(&key.node, &key.span, out);
                collect_action_calls(&val.node, &val.span, out);
            }
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements {
                collect_action_calls(&el.node, &el.span, out);
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            collect_action_calls(&start.node, &start.span, out);
            collect_action_calls(&step.node, &step.span, out);
            collect_action_calls(&end.node, &end.span, out);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            collect_action_calls(&iterable.node, &iterable.span, out);
            for stmt in body {
                collect_action_calls(&stmt.node, &stmt.span, out);
            }
        }
        TypedExprKind::In { item, collection } => {
            collect_action_calls(&item.node, &item.span, out);
            collect_action_calls(&collection.node, &collection.span, out);
        }
        // ── Map/filter/fold — recursão ──
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
            collect_action_calls(&callback.node, &callback.span, out);
            collect_action_calls(&collection.node, &collection.span, out);
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            collect_action_calls(&callback.node, &callback.span, out);
            collect_action_calls(&initial.node, &initial.span, out);
            collect_action_calls(&collection.node, &collection.span, out);
        }
        // ── FusedStream — recursão ──
        TypedExprKind::FusedStream { stages, source, .. } => {
            collect_action_calls(&source.node, &source.span, out);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                collect_action_calls(&cb.node, &cb.span, out);
            }
        }
        // ── CSP — recursão ──
        TypedExprKind::ChannelSend { channel, value } => {
            collect_action_calls(&channel.node, &channel.span, out);
            collect_action_calls(&value.node, &value.span, out);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            collect_action_calls(&channel.node, &channel.span, out);
        }
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            for arm in arms {
                collect_action_calls(&arm.channel.node, &arm.channel.span, out);
                collect_action_calls(&arm.body.node, &arm.body.span, out);
            }
            if let Some(tm) = timeout_ms {
                collect_action_calls(&tm.node, &tm.span, out);
            }
            if let Some(tb) = timeout_body {
                collect_action_calls(&tb.node, &tb.span, out);
            }
        }
        // ChannelCreate não tem sub-expressões (args já foram consumidos pelo typeck).
        TypedExprKind::ChannelCreate { .. } => {}
        // ReceiverFactoryCall: o factory é uma sub-expressão (Ident do rxf).
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            collect_action_calls(&factory.node, &factory.span, out);
        }
        // Fork spawna uma Action — registra no call graph.
        TypedExprKind::Fork { action_name, action_expr, args } => {
            if action_name != "__indirect_fork" {
                out.push((action_name.clone(), expr.span));
            }
            collect_action_calls(&action_expr.node, &action_expr.span, out);
            collect_action_calls(&args.node, &args.span, out);
        }
        // Folhas sem sub-expressões.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::VariantConstruct { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
    }
}

/// Coleta ActionCalls de um braço de match (pattern + guard + body).
fn collect_match_arm_calls(arm: &TypedMatchArm, out: &mut Vec<(String, kata_ast::Span)>) {
    // Patterns não contêm expressões (só literals/sub-patterns), mas
    // Literal pattern contém um TypedExpr — pode ter ActionCall?
    // Não: patterns são passivos (não executam), mas percorremos por segurança.
    if let Some(pattern) = &arm.pattern {
        collect_pattern_calls(&pattern.node, &pattern.span, out);
    }
    if let Some(guard) = &arm.guard {
        collect_action_calls(&guard.node, &guard.span, out);
    }
    collect_action_calls(&arm.body.node, &arm.body.span, out);
}

/// Coleta ActionCalls de uma cláusula lambda (guards + with bindings + body).
fn collect_lambda_clause_calls(
    clause: &TypedLambdaClause,
    out: &mut Vec<(String, kata_ast::Span)>,
) {
    for guard in &clause.guards {
        if let Some(cond) = &guard.condition {
            collect_action_calls(&cond.node, &cond.span, out);
        }
        collect_action_calls(&guard.body.node, &guard.body.span, out);
    }
    for wb in &clause.with_bindings {
        collect_action_calls(&wb.value.node, &wb.value.span, out);
    }
    if clause.guards.is_empty() {
        collect_action_calls(&clause.body.node, &clause.body.span, out);
    }
}

/// Coleta ActionCalls dentro de patterns (literal patterns podem conter exprs).
fn collect_pattern_calls(
    pattern: &TypedPattern,
    _span: &kata_ast::Span,
    out: &mut Vec<(String, kata_ast::Span)>,
) {
    match pattern {
        TypedPattern::Literal { value } => {
            collect_action_calls(&value.node, &value.span, out);
        }
        TypedPattern::Variant {
            sub_patterns: Some(subs),
            ..
        } => {
            for sub in subs {
                collect_pattern_calls(&sub.node, &sub.span, out);
            }
        }
        TypedPattern::Tuple { elements } => {
            for el in elements {
                collect_pattern_calls(&el.node, &el.span, out);
            }
        }
        TypedPattern::Cons { head, tail } => {
            collect_pattern_calls(&head.node, &head.span, out);
            collect_pattern_calls(&tail.node, &tail.span, out);
        }
        // Ident, Wildcard, Variant sem sub_patterns, Nil — sem sub-exprs.
        TypedPattern::Ident { .. }
        | TypedPattern::Wildcard
        | TypedPattern::Variant {
            sub_patterns: None, ..
        }
        | TypedPattern::Nil => {}
    }
}

/// Color do DFS cycle-detection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    White,
    Gray,
    Black,
}

/// DFS com coloring para detectar ciclos no call graph.
///
/// - White: não visitado.
/// - Gray: em progresso (na stack atual do DFS).
/// - Black: finalizado (todos descendentes explorados).
///
/// Se ao visitar um nó Gray encontramos outro Gray, há um ciclo.
fn detect_cycle(graph: &HashMap<String, Vec<(String, kata_ast::Span)>>) -> Result<(), MiddleError> {
    let mut colors: HashMap<String, Color> = HashMap::new();
    for name in graph.keys() {
        colors.insert(name.clone(), Color::White);
    }

    for start in graph.keys() {
        if colors.get(start) == Some(&Color::White) {
            let mut path: Vec<String> = Vec::new();
            dfs_visit(start, graph, &mut colors, &mut path)?;
        }
    }

    Ok(())
}

/// Visita um nó no DFS. Se encontrar um nó Gray, reporta o ciclo.
fn dfs_visit(
    node: &str,
    graph: &HashMap<String, Vec<(String, kata_ast::Span)>>,
    colors: &mut HashMap<String, Color>,
    path: &mut Vec<String>,
) -> Result<(), MiddleError> {
    colors.insert(node.to_string(), Color::Gray);
    path.push(node.to_string());

    if let Some(callees) = graph.get(node) {
        for (callee, span) in callees {
            let callee_color = colors.get(callee).copied().unwrap_or(Color::White);

            // Callee não está no grafo (não é uma Action definida pelo
            // usuário — ex: função pura chamada como Action por engano,
            // ou Action do prelude). Ignorar.
            if !graph.contains_key(callee) {
                continue;
            }

            match callee_color {
                Color::Gray => {
                    // Ciclo encontrado: o callee está na stack atual.
                    // Constrói a chain do ciclo.
                    let cycle_start_idx = path.iter().position(|n| n == callee);
                    let mut chain = if let Some(idx) = cycle_start_idx {
                        path[idx..].to_vec()
                    } else {
                        path.clone()
                    };
                    chain.push(callee.clone()); // fecha o ciclo: A → B → A

                    let cycle_str = chain.join(" → ");

                    return Err(MiddleError::RecursiveAction {
                        action: callee.clone(),
                        cycle: cycle_str,
                        span: (*span).into(),
                    });
                }
                Color::White => {
                    dfs_visit(callee, graph, colors, path)?;
                    // Após retornar, se path encolheu, sincronizar.
                    // (path é mutável e dfs_visit faz push/pop)
                }
                Color::Black => {
                    // Já finalizado — sem ciclo por aqui.
                }
            }
        }
    }

    path.pop();
    colors.insert(node.to_string(), Color::Black);
    Ok(())
}
