//! Parser: Dict and Set literals.

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item};

#[test]
fn dict_lit_basic() {
    let m = parse_src("{\"a\": 1 \"b\": 2}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::DictLit { entries } => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].0.node, Expr::TextLit { text: "a".into() });
                assert_eq!(entries[0].1.node, Expr::IntLit { text: "1".into() });
                assert_eq!(entries[1].0.node, Expr::TextLit { text: "b".into() });
                assert_eq!(entries[1].1.node, Expr::IntLit { text: "2".into() });
            }
            other => panic!("expected DictLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn dict_lit_empty() {
    let m = parse_src("{:}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::DictLit { entries } => assert!(entries.is_empty()),
            other => panic!("expected DictLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn set_lit_basic() {
    let m = parse_src("{|1 2 3|}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::SetLit { elements } => {
                assert_eq!(elements.len(), 3);
                assert_eq!(elements[0].node, Expr::IntLit { text: "1".into() });
                assert_eq!(elements[1].node, Expr::IntLit { text: "2".into() });
                assert_eq!(elements[2].node, Expr::IntLit { text: "3".into() });
            }
            other => panic!("expected SetLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn set_lit_empty() {
    let m = parse_src("{||}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::SetLit { elements } => assert!(elements.is_empty()),
            other => panic!("expected SetLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn array_lit_still_works() {
    let m = parse_src("{1 2 3}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ArrayLit { elements } => assert_eq!(elements.len(), 3),
            other => panic!("expected ArrayLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn array_empty_still_works() {
    let m = parse_src("{}");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ArrayLit { elements } => assert!(elements.is_empty()),
            other => panic!("expected ArrayLit, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}
