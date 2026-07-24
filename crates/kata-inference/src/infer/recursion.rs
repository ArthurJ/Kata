//! Proibição de recursão em Actions.
//!
//! Actions executam em fibers com stack fixa (1 MB). Recursão — direta ou
//! indireta — poderia estourar a stack sem o programador perceber. Esta
//! análise constrói o call graph das Actions e rejeita ciclos via DFS.
//!
//! Só Actions do usuário (`ffi_symbol: None`) participam do call graph.
//! Builtins (`echo!`, `panic!`, `assert!`) têm `ffi_symbol: Some(...)` e
//! são ignorados — não são Actions recursivas.

use std::collections::{HashMap, HashSet};

use kata_ast::Span;
use kata_core::ty::Ty;
use kata_diagnostics::MiddleError;

use crate::typed::{TypedAction, TypedExpr, TypedExprKind};

use super::cycle::detect_cycle;
use super::walk::for_each_subexpr;

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

// ── Call graph direto (ActionCall + Fork) ───────────────────────────

/// Constrói o call graph: `Action name → Vec<callee names>`.
///
/// Só inclui `ActionCall` com `ffi_symbol: None` (Actions do usuário).
/// Builtins são ignorados.
fn build_call_graph(actions: &[TypedAction]) -> HashMap<String, Vec<(String, Span)>> {
    let mut graph: HashMap<String, Vec<(String, Span)>> = HashMap::new();
    for action in actions {
        let mut callees = Vec::new();
        for stmt in &action.body {
            collect_action_calls(&stmt.node, &mut callees);
        }
        graph.insert(action.name.clone(), callees);
    }
    graph
}

/// Coleta recursivamente todos os `ActionCall` com `ffi_symbol: None`
/// e `Fork` (com action_name != "__indirect_fork") na árvore da TAST.
fn collect_action_calls(expr: &TypedExpr, out: &mut Vec<(String, Span)>) {
    for_each_subexpr(expr, &mut |e| {
        match &e.kind {
            TypedExprKind::ActionCall {
                callee,
                ffi_symbol: None,
                ..
            } => {
                out.push((callee.clone(), e.span));
            }
            TypedExprKind::Fork { action_name, .. } if action_name != "__indirect_fork" => {
                out.push((action_name.clone(), e.span));
            }
            _ => {}
        }
        true
    });
}

// ── Call graph indireto (callable params / first-class Action refs) ──

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
    graph: &mut HashMap<String, Vec<(String, Span)>>,
) {
    // Conjunto dos nomes de Actions definidas pelo usuário — usado para
    // distinguir first-class Action reference de variável local comum.
    let action_names: HashSet<&str> = actions.iter().map(|a| a.name.as_str()).collect();

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
        let invoked_params = collect_invoked_callable_params(&target.body, &callable_params);
        if invoked_params.is_empty() {
            continue;
        }

        // 3. Encontra call sites de target nas outras actions (e em si
        //    mesma — recursão indireta via param é possível).
        for caller in actions {
            for stmt in &caller.body {
                let edges = find_call_sites_of(&stmt.node, &target.name, &action_names);
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
/// matches um dos nomes em `callable_params`. Retorna os nomes dos params
/// invocados (sem duplicar).
fn collect_invoked_callable_params(
    stmts: &[kata_ast::Spanned<TypedExpr>],
    callable_params: &[&str],
) -> Vec<String> {
    let mut invoked: Vec<String> = Vec::new();
    let param_set: HashSet<&str> = callable_params.iter().copied().collect();

    for stmt in stmts {
        for_each_subexpr(&stmt.node, &mut |e| {
            if let TypedExprKind::ActionCall {
                callee,
                indirect_callee: Some(_),
                ffi_symbol: None,
                ..
            } = &e.kind
                && param_set.contains(callee.as_str())
                && !invoked.iter().any(|p: &String| p == callee)
            {
                invoked.push(callee.clone());
            }
            true
        });
    }

    invoked
}

/// Encontra call sites de `target_name` na expressão. Para cada call site,
/// retorna `(span, Vec<arg_action_name>)` — a lista de args que são
/// first-class Action references (Ident com nome de Action definida).
fn find_call_sites_of(
    expr: &TypedExpr,
    target_name: &str,
    action_names: &HashSet<&str>,
) -> Vec<(Span, Vec<String>)> {
    let mut out = Vec::new();

    for_each_subexpr(expr, &mut |e| {
        match &e.kind {
            TypedExprKind::ActionCall {
                callee,
                args,
                ffi_symbol: None,
                ..
            } if callee == target_name => {
                let arg_actions = extract_action_idents_from_args(&args.node, action_names);
                if !arg_actions.is_empty() {
                    out.push((e.span, arg_actions));
                }
            }
            TypedExprKind::Fork {
                action_name, args, ..
            } if action_name == target_name => {
                let arg_actions = extract_action_idents_from_args(&args.node, action_names);
                if !arg_actions.is_empty() {
                    out.push((e.span, arg_actions));
                }
            }
            _ => {}
        }
        true
    });

    out
}

/// Extrai Ident names que são Actions definidas pelo usuário da tupla
/// de args de um call site. Aparecem como elementos de Tuple (calls
/// com mais de 1 arg) ou como a própria expr (call com 1 arg).
fn extract_action_idents_from_args(
    args_expr: &TypedExpr,
    action_names: &HashSet<&str>,
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
fn extract_action_idents(expr: &TypedExpr, action_names: &HashSet<&str>, out: &mut Vec<String>) {
    if let TypedExprKind::Ident { name } = &expr.kind
        && action_names.contains(name.as_str())
    {
        out.push(name.clone());
    }
}
