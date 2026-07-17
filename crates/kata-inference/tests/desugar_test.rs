//! Integration tests for `desugar` pass (Fase 7 — Fio 2).
//!
//! Testa `desugar::desugar` como função pura sobre `Spanned<Expr>`: verifica
//! que Hole e Pipe são totalmente eliminados da AST. Não passa pelo
//! inference completo (lambda/match são Fase 8 — testamos só a transformação).

use kata_ast::{Expr, Pattern, Spanned};
use kata_inference::desugar;
use kata_lexer::lex;
use kata_parser::parse;

mod common;

use common::{assert_no_holes, assert_no_pipes};

// ── Helpers ───────────────────────────────────────────────────────

/// Lexa + parseia src e retorna o EntryExpr como Spanned<Expr>.
fn parse_entry(src: &str) -> Spanned<Expr> {
    let tokens = lex(src).unwrap_or_else(|e| panic!("lex failed: {e:?}"));
    let module = parse(tokens).unwrap_or_else(|e| panic!("parse failed: {e:?}"));
    module
        .items
        .into_iter()
        .find_map(|item| match item.node {
            kata_ast::Item::EntryExpr(expr) => Some(expr),
            _ => None,
        })
        .expect("módulo deve ter EntryExpr")
}

// `assert_no_holes` e `assert_no_pipes` (traversal recursivo sobre Expr) vivem
// em [`common`] — reusáveis por outros test files.

// ── Pipe: eliminação total ─────────────────────────────────────────

#[test]
fn desugar_pipe_with_hole() {
    // 5 |> + 10 _ → + 10 5
    let entry = parse_entry("5 |> + 10 _");
    let result = desugar::desugar(&entry);
    assert_no_pipes(&result);
    assert_no_holes(&result);
    // Esperado: Apply { callee: Ident("+"), args: [IntLit 10, IntLit 5] }
    match &result.node {
        Expr::Apply { callee, args } => {
            assert!(matches!(&callee.node, Expr::Ident { name } if name == "+"));
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0].node, Expr::IntLit { text } if text == "10"));
            assert!(matches!(&args[1].node, Expr::IntLit { text } if text == "5"));
        }
        other => panic!("esperado Apply, got {other:?}"),
    }
}

#[test]
fn desugar_pipe_hole_first_position() {
    // 5 |> + _ 10 → + 5 10
    let entry = parse_entry("5 |> + _ 10");
    let result = desugar::desugar(&entry);
    assert_no_pipes(&result);
    assert_no_holes(&result);
    match &result.node {
        Expr::Apply { callee, args } => {
            assert!(matches!(&callee.node, Expr::Ident { name } if name == "+"));
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0].node, Expr::IntLit { text } if text == "5"));
            assert!(matches!(&args[1].node, Expr::IntLit { text } if text == "10"));
        }
        other => panic!("esperado Apply, got {other:?}"),
    }
}

#[test]
fn desugar_pipe_without_hole_injects_first_arg() {
    // 5 |> + 10 1 → + 5 10 1 (injeta lhs como primeiro arg)
    let entry = parse_entry("5 |> + 10 1");
    let result = desugar::desugar(&entry);
    assert_no_pipes(&result);
    // Esperado: Apply { callee: Ident("+"), args: [5, 10, 1] }
    match &result.node {
        Expr::Apply { callee, args } => {
            assert!(matches!(&callee.node, Expr::Ident { name } if name == "+"));
            assert_eq!(args.len(), 3);
            assert!(matches!(&args[0].node, Expr::IntLit { text } if text == "5"));
            assert!(matches!(&args[1].node, Expr::IntLit { text } if text == "10"));
            assert!(matches!(&args[2].node, Expr::IntLit { text } if text == "1"));
        }
        other => panic!("esperado Apply, got {other:?}"),
    }
}

#[test]
fn desugar_pipe_left_assoc_chain() {
    // 5 |> + 1 _ |> * 2 _ → (* 2 (+ 1 5))
    // Passo 1: 5 |> + 1 _ → + 1 5
    // Passo 2: (+ 1 5) |> * 2 _ → * 2 (+ 1 5)
    let entry = parse_entry("5 |> + 1 _ |> * 2 _");
    let result = desugar::desugar(&entry);
    assert_no_pipes(&result);
    assert_no_holes(&result);
    // Esperado: Apply { callee: Ident("*"), args: [IntLit 2, Apply(+, [1, 5])] }
    match &result.node {
        Expr::Apply { callee, args } => {
            assert!(matches!(&callee.node, Expr::Ident { name } if name == "*"));
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0].node, Expr::IntLit { text } if text == "2"));
            match &args[1].node {
                Expr::Apply { callee, args } => {
                    assert!(matches!(&callee.node, Expr::Ident { name } if name == "+"));
                    assert_eq!(args.len(), 2);
                    assert!(matches!(&args[0].node, Expr::IntLit { text } if text == "1"));
                    assert!(matches!(&args[1].node, Expr::IntLit { text } if text == "5"));
                }
                other => panic!("esperado Apply inner, got {other:?}"),
            }
        }
        other => panic!("esperado Apply outer, got {other:?}"),
    }
}

#[test]
fn desugar_pipe_to_bare_ident() {
    // 5 |> inc → inc 5
    // Mas "inc" não existe no prelude — só testamos a transformação AST.
    let entry = parse_entry("42 |> show");
    let result = desugar::desugar(&entry);
    assert_no_pipes(&result);
    match &result.node {
        Expr::Apply { callee, args } => {
            assert!(matches!(&callee.node, Expr::Ident { name } if name == "show"));
            assert_eq!(args.len(), 1);
            assert!(matches!(&args[0].node, Expr::IntLit { text } if text == "42"));
        }
        other => panic!("esperado Apply, got {other:?}"),
    }
}

// ── Hole: eliminação total via Lambda ──────────────────────────────

#[test]
fn desugar_single_hole_becomes_lambda() {
    // + 10 _ → lambda __hole_0: + 10 __hole_0
    let entry = parse_entry("+ 10 _");
    let result = desugar::desugar(&entry);
    assert_no_holes(&result);
    match &result.node {
        Expr::Lambda { patterns, body, .. } => {
            assert_eq!(patterns.len(), 1);
            assert!(matches!(
                &patterns[0].node,
                Pattern::Ident(name) if name == "__hole_0"
            ));
            // Body deve ser Apply { +, [10, __hole_0] }
            match &body.node {
                Expr::Apply { callee, args } => {
                    assert!(matches!(&callee.node, Expr::Ident { name } if name == "+"));
                    assert_eq!(args.len(), 2);
                    assert!(matches!(&args[0].node, Expr::IntLit { text } if text == "10"));
                    assert!(matches!(&args[1].node, Expr::Ident { name } if name == "__hole_0"));
                }
                other => panic!("esperado Apply no body, got {other:?}"),
            }
        }
        other => panic!("esperado Lambda, got {other:?}"),
    }
}

#[test]
fn desugar_hole_first_position() {
    // - _ 10 → lambda __hole_0: - __hole_0 10
    let entry = parse_entry("- _ 10");
    let result = desugar::desugar(&entry);
    assert_no_holes(&result);
    match &result.node {
        Expr::Lambda { patterns, body, .. } => {
            assert_eq!(patterns.len(), 1);
            assert!(matches!(
                &patterns[0].node,
                Pattern::Ident(name) if name == "__hole_0"
            ));
            match &body.node {
                Expr::Apply { callee, args } => {
                    assert!(matches!(&callee.node, Expr::Ident { name } if name == "-"));
                    assert_eq!(args.len(), 2);
                    assert!(matches!(&args[0].node, Expr::Ident { name } if name == "__hole_0"));
                    assert!(matches!(&args[1].node, Expr::IntLit { text } if text == "10"));
                }
                other => panic!("esperado Apply no body, got {other:?}"),
            }
        }
        other => panic!("esperado Lambda, got {other:?}"),
    }
}

#[test]
fn desugar_two_holes_become_two_params() {
    // + _ _ → lambda __hole_0 __hole_1: + __hole_0 __hole_1
    let entry = parse_entry("+ _ _");
    let result = desugar::desugar(&entry);
    assert_no_holes(&result);
    match &result.node {
        Expr::Lambda { patterns, body, .. } => {
            assert_eq!(patterns.len(), 2);
            assert!(matches!(
                &patterns[0].node,
                Pattern::Ident(name) if name == "__hole_0"
            ));
            assert!(matches!(
                &patterns[1].node,
                Pattern::Ident(name) if name == "__hole_1"
            ));
            match &body.node {
                Expr::Apply { callee, args } => {
                    assert!(matches!(&callee.node, Expr::Ident { name } if name == "+"));
                    assert_eq!(args.len(), 2);
                    assert!(matches!(&args[0].node, Expr::Ident { name } if name == "__hole_0"));
                    assert!(matches!(&args[1].node, Expr::Ident { name } if name == "__hole_1"));
                }
                other => panic!("esperado Apply no body, got {other:?}"),
            }
        }
        other => panic!("esperado Lambda, got {other:?}"),
    }
}

#[test]
fn desugar_no_hole_unchanged() {
    // + 10 5 — sem hole, desugar não produz Lambda
    let entry = parse_entry("+ 10 5");
    let result = desugar::desugar(&entry);
    assert_no_holes(&result);
    match &result.node {
        Expr::Apply { callee, args } => {
            assert!(matches!(&callee.node, Expr::Ident { name } if name == "+"));
            assert_eq!(args.len(), 2);
        }
        other => panic!("esperado Apply, got {other:?}"),
    }
}

// ── Combinação: Pipe + Hole ───────────────────────────────────────

#[test]
fn desugar_pipe_with_hole_leaves_remaining_hole_as_lambda() {
    // 5 |> + _ _ → pipe substitui primeiro Hole por 5, depois desugar_holes
    // cria Lambda para o Hole restante.
    // Resultado: lambda __hole_0: + 5 __hole_0
    let entry = parse_entry("5 |> + _ _");
    let result = desugar::desugar(&entry);
    assert_no_pipes(&result);
    assert_no_holes(&result);
    match &result.node {
        Expr::Lambda { patterns, body, .. } => {
            assert_eq!(patterns.len(), 1);
            assert!(matches!(
                &patterns[0].node,
                Pattern::Ident(name) if name == "__hole_0"
            ));
            match &body.node {
                Expr::Apply { callee, args } => {
                    assert!(matches!(&callee.node, Expr::Ident { name } if name == "+"));
                    assert_eq!(args.len(), 2);
                    assert!(matches!(&args[0].node, Expr::IntLit { text } if text == "5"));
                    assert!(matches!(&args[1].node, Expr::Ident { name } if name == "__hole_0"));
                }
                other => panic!("esperado Apply no body, got {other:?}"),
            }
        }
        other => panic!("esperado Lambda, got {other:?}"),
    }
}

// ── Idempotência ──────────────────────────────────────────────────

#[test]
fn desugar_idempotent_no_hole_no_pipe() {
    // Expressão sem Hole/Pipe — desugar não muda nada (exceto clones)
    let entry = parse_entry("+ 1 2");
    let result1 = desugar::desugar(&entry);
    let result2 = desugar::desugar(&result1);
    // Estrutura deve ser idêntica
    assert_eq!(result1.node, result2.node);
}

#[test]
fn desugar_preserves_let() {
    // let x := 5 — Kata não tem `in`, let é só `let x := <expr>`
    let entry = parse_entry("let x := 5");
    let result = desugar::desugar(&entry);
    assert_no_holes(&result);
    assert_no_pipes(&result);
    // Deve ainda ter um Let
    match &result.node {
        Expr::Let { name, value } => {
            assert_eq!(name, "x");
            assert!(matches!(&value.node, Expr::IntLit { text } if text == "5"));
        }
        other => panic!("esperado Let, got {other:?}"),
    }
}
