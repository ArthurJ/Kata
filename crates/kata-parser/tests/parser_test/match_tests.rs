//! Match expression parsing: boolean arms, otherwise, variant qual,
//! literal, and tuple patterns.

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item};

#[test]
fn match_boolean_two_arms() {
    let m = parse_src("match = 1 1\n    True: \"igual\"\n    False: \"diferente\"");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Match { scrutinee, arms } => {
                // scrutinee é `= 1 1`
                match &scrutinee.node {
                    Expr::Apply { callee, args } => {
                        assert_eq!(callee.node, Expr::Ident { name: "=".into() });
                        assert_eq!(args.len(), 2);
                    }
                    other => panic!("expected Apply scrutinee, got {other:?}"),
                }
                assert_eq!(arms.len(), 2);
                // First arm: True: "igual"
                match &arms[0].pattern {
                    Some(p) => match &p.node {
                        kata_ast::Pattern::Ident(name) => assert_eq!(name, "True"),
                        other => panic!("expected Ident pattern, got {other:?}"),
                    },
                    None => panic!("expected Some pattern"),
                }
                match &arms[0].body.node {
                    Expr::TextLit { text } => assert_eq!(text, "igual"),
                    other => panic!("expected TextLit body, got {other:?}"),
                }
                // Second arm: False: "diferente"
                match &arms[1].pattern {
                    Some(p) => match &p.node {
                        kata_ast::Pattern::Ident(name) => assert_eq!(name, "False"),
                        other => panic!("expected Ident pattern, got {other:?}"),
                    },
                    None => panic!("expected Some pattern"),
                }
            }
            other => panic!("expected Match, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Match), got {other:?}"),
    }
}

#[test]
fn match_with_otherwise() {
    let m = parse_src("match Boolean::True\n    True: 1\n    otherwise: 0");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Match { arms, .. } => {
                assert_eq!(arms.len(), 2);
                // First arm: True: 1
                assert!(arms[0].pattern.is_some());
                // Second arm: otherwise: 0
                assert!(arms[1].pattern.is_none());
            }
            other => panic!("expected Match, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Match), got {other:?}"),
    }
}

#[test]
fn match_variant_qual_pattern() {
    let m = parse_src("match Boolean::True\n    Boolean::True: 1\n    Boolean::False: 0");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Match { arms, .. } => {
                assert_eq!(arms.len(), 2);
                match &arms[0].pattern.as_ref().unwrap().node {
                    kata_ast::Pattern::Variant {
                        enum_name, variant, ..
                    } => {
                        assert_eq!(enum_name, "Boolean");
                        assert_eq!(variant, "True");
                    }
                    other => panic!("expected Variant pattern, got {other:?}"),
                }
            }
            other => panic!("expected Match, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Match), got {other:?}"),
    }
}

#[test]
fn match_literal_pattern() {
    let m = parse_src("match 5\n    0: \"zero\"\n    otherwise: \"não-zero\"");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Match { arms, .. } => {
                assert_eq!(arms.len(), 2);
                match &arms[0].pattern.as_ref().unwrap().node {
                    kata_ast::Pattern::Literal(expr) => {
                        assert_eq!(expr.node, Expr::IntLit { text: "0".into() });
                    }
                    other => panic!("expected Literal pattern, got {other:?}"),
                }
            }
            other => panic!("expected Match, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Match), got {other:?}"),
    }
}

#[test]
fn match_tuple_pattern() {
    let m = parse_src("match (1, 2)\n    (a, b): a");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Match { arms, .. } => {
                assert_eq!(arms.len(), 1);
                match &arms[0].pattern.as_ref().unwrap().node {
                    kata_ast::Pattern::Tuple(elements) => {
                        assert_eq!(elements.len(), 2);
                        assert_eq!(elements[0].node, kata_ast::Pattern::Ident("a".into()));
                        assert_eq!(elements[1].node, kata_ast::Pattern::Ident("b".into()));
                    }
                    other => panic!("expected Tuple pattern, got {other:?}"),
                }
            }
            other => panic!("expected Match, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Match), got {other:?}"),
    }
}
