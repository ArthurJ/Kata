//! Detecção de ciclos no call graph via DFS com coloring.
//!
//! Extraído de `recursion.rs` — contém apenas o algoritmo de cycle
//! detection (white/gray/black), independente da construção do grafo.

use std::collections::HashMap;

use kata_ast::Span;
use kata_diagnostics::MiddleError;

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
/// Retorna `RecursiveAction` no primeiro ciclo encontrado, com a chain
/// (ex: "A → B → A") e o span do `ActionCall` que fecha o ciclo.
pub(crate) fn detect_cycle(
    graph: &HashMap<String, Vec<(String, Span)>>,
) -> Result<(), MiddleError> {
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
    graph: &HashMap<String, Vec<(String, Span)>>,
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