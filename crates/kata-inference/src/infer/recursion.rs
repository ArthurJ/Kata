//! Fase 11 — Proibição de recursão em Actions.
//!
//! Actions executam em fibers com stack fixa (1 MB). Recursão — direta ou
//! indireta — poderia estourar a stack sem o programador perceber. Esta
//! análise constrói o call graph das Actions e rejeita ciclos via DFS.
//!
//! Só Actions do usuário (`ffi_symbol: None`) participam do call graph.
//! Builtins (`echo!`, `panic!`, `assert!`) têm `ffi_symbol: Some(...)` e
//! são ignorados — não são Actions recursivas.

use std::collections::HashMap;

use kata_diagnostics::MiddleError;

use crate::typed::{
    TypedAction, TypedExpr, TypedExprKind, TypedLambdaClause, TypedMatchArm, TypedPattern,
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
    let call_graph = build_call_graph(actions);
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
        TypedExprKind::Let { value, .. } | TypedExprKind::Var { value, .. } => {
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
        // ── Fio 8: Coleções — recursão nos elementos ──
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
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
        // Ident, Wildcard, Variant sem sub_patterns — sem sub-exprs.
        TypedPattern::Ident { .. }
        | TypedPattern::Wildcard
        | TypedPattern::Variant {
            sub_patterns: None, ..
        } => {}
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
