//! Fio 8 — Parser: List/Array/Range literals (DoDs 5-12).

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item};

#[test]
fn list_lit_basic() {
    // DoD 5: `[1 2 3]` → ListLit com 3 elementos
    let m = parse_src("[1 2 3]");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ListLit { elements } => {
                assert_eq!(elements.len(), 3);
                assert_eq!(elements[0].node, Expr::IntLit { text: "1".into() });
                assert_eq!(elements[1].node, Expr::IntLit { text: "2".into() });
                assert_eq!(elements[2].node, Expr::IntLit { text: "3".into() });
            }
            other => panic!("expected ListLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn array_lit_basic() {
    // DoD 6: `{1 2 3}` → ArrayLit com 3 elementos
    let m = parse_src("{1 2 3}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ArrayLit { elements } => {
                assert_eq!(elements.len(), 3);
                assert_eq!(elements[0].node, Expr::IntLit { text: "1".into() });
                assert_eq!(elements[1].node, Expr::IntLit { text: "2".into() });
                assert_eq!(elements[2].node, Expr::IntLit { text: "3".into() });
            }
            other => panic!("expected ArrayLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn range_lit_exclusive() {
    // DoD 7: `[0..1..10]` → RangeLit { start=0, step=1, end=10, inclusive=false }
    let m = parse_src("[0..1..10]");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::RangeLit {
                start,
                step,
                end,
                inclusive,
            } => {
                assert_eq!(start.node, Expr::IntLit { text: "0".into() });
                assert_eq!(step.node, Expr::IntLit { text: "1".into() });
                assert_eq!(end.node, Expr::IntLit { text: "10".into() });
                assert!(!inclusive);
            }
            other => panic!("expected RangeLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn range_lit_inclusive() {
    // DoD 8: `[0..1..=10]` → RangeLit { start=0, step=1, end=10, inclusive=true }
    let m = parse_src("[0..1..=10]");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::RangeLit {
                start,
                step,
                end,
                inclusive,
            } => {
                assert_eq!(start.node, Expr::IntLit { text: "0".into() });
                assert_eq!(step.node, Expr::IntLit { text: "1".into() });
                assert_eq!(end.node, Expr::IntLit { text: "10".into() });
                assert!(inclusive);
            }
            other => panic!("expected RangeLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn range_lit_step_2_exclusive() {
    // DoD 9: `[0..2..10]` → RangeLit { start=0, step=2, end=10, inclusive=false }
    let m = parse_src("[0..2..10]");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::RangeLit {
                start,
                step,
                end,
                inclusive,
            } => {
                assert_eq!(start.node, Expr::IntLit { text: "0".into() });
                assert_eq!(step.node, Expr::IntLit { text: "2".into() });
                assert_eq!(end.node, Expr::IntLit { text: "10".into() });
                assert!(!inclusive);
            }
            other => panic!("expected RangeLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn range_lit_step_2_inclusive() {
    // DoD 10: `[0..2..=10]` → RangeLit { start=0, step=2, end=10, inclusive=true }
    let m = parse_src("[0..2..=10]");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::RangeLit {
                start,
                step,
                end,
                inclusive,
            } => {
                assert_eq!(start.node, Expr::IntLit { text: "0".into() });
                assert_eq!(step.node, Expr::IntLit { text: "2".into() });
                assert_eq!(end.node, Expr::IntLit { text: "10".into() });
                assert!(inclusive);
            }
            other => panic!("expected RangeLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn range_lit_float() {
    // DoD 11: `[0.0..0.1..1.0]` → RangeLit com elementos Float
    let m = parse_src("[0.0..0.1..1.0]");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::RangeLit {
                start,
                step,
                end,
                inclusive,
            } => {
                assert_eq!(start.node, Expr::FloatLit { text: "0.0".into() });
                assert_eq!(step.node, Expr::FloatLit { text: "0.1".into() });
                assert_eq!(end.node, Expr::FloatLit { text: "1.0".into() });
                assert!(!inclusive);
            }
            other => panic!("expected RangeLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn list_lit_empty() {
    // DoD 12: `[]` → ListLit com 0 elementos
    let m = parse_src("[]");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ListLit { elements } => {
                assert!(elements.is_empty());
            }
            other => panic!("expected ListLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn array_lit_empty() {
    // Extra: `{}` → ArrayLit com 0 elementos
    let m = parse_src("{}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ArrayLit { elements } => {
                assert!(elements.is_empty());
            }
            other => panic!("expected ArrayLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── Fase 3: for x in + operador in (DoDs 13-16) ──────────────────

use kata_lexer::lex;
use kata_parser::parse;

#[test]
fn for_in_parses_inside_action() {
    // DoD 13: `for x in arr` parseia e produz Expr::ForIn
    let src = "action iterar\n    for x in arr\n        echo!(x)";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => {
            assert_eq!(body.len(), 1);
            match &body[0].expr.node {
                Expr::ForIn {
                    var_name,
                    iterable,
                    body: for_body,
                } => {
                    assert_eq!(var_name, "x");
                    assert!(matches!(&iterable.node, Expr::Ident { name } if name == "arr"));
                    assert_eq!(for_body.len(), 1);
                    assert!(matches!(&for_body[0].node, Expr::ActionCall { callee, .. } if callee == "echo"));
                }
                other => panic!("expected ForIn, got {other:?}"),
            }
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn for_in_outside_action_errors() {
    // DoD 14: `for` fora de Action produz erro
    let tokens = lex("for x in arr\n    echo!(x)").unwrap();
    let result = parse(tokens);
    assert!(result.is_err(), "for fora de Action deve produzir erro");
}

#[test]
fn for_in_in_lambda_errors() {
    // DoD 15: `for` em lambda produz compile error (parser rejeita —
    // `for` não é aceito em lambda body porque não está em action_body)
    let src = "lambda x: for y in x\n    echo!(y)";
    let tokens = lex(src).unwrap();
    let result = parse(tokens);
    assert!(result.is_err(), "for em lambda deve produzir erro");
}

#[test]
fn in_operator_parses() {
    // DoD 16: `3 in {1 2 3}` parseia e produz Expr::In
    let m = parse_src("3 in {1 2 3}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::In { item, collection } => {
                assert_eq!(item.node, Expr::IntLit { text: "3".into() });
                match &collection.node {
                    Expr::ArrayLit { elements } => {
                        assert_eq!(elements.len(), 3);
                    }
                    other => panic!("expected ArrayLit in collection, got {other:?}"),
                }
            }
            other => panic!("expected In, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}