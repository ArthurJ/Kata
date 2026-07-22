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

#[test]
fn match_unqualified_variant_with_payload() {
    // `Ok v:` em match arm — variante desqualificada com payload.
    // O parser deve produzir Pattern::Variant com enum_name vazio.
    // O typeck resolve enum_name via EnumRegistry do scrutinee.
    let m = parse_src("match Optional::Some 42\n    Some n: n\n    None: 0\n    otherwise: 0");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Match { arms, .. } => {
                assert_eq!(arms.len(), 3);
                // First arm: Some n — unqualified variant with payload
                match &arms[0].pattern.as_ref().unwrap().node {
                    kata_ast::Pattern::Variant {
                        enum_name,
                        variant,
                        payload,
                    } => {
                        assert_eq!(
                            enum_name, "",
                            "unqualified variant should have empty enum_name"
                        );
                        assert_eq!(variant, "Some");
                        assert!(payload.is_some(), "Some should have payload");
                        assert_eq!(payload.as_ref().unwrap().len(), 1);
                    }
                    other => panic!("expected Variant pattern for `Some n`, got {other:?}"),
                }
                // Second arm: None — unqualified unit variant (Ident, typeck resolves)
                match &arms[1].pattern.as_ref().unwrap().node {
                    kata_ast::Pattern::Ident(name) => assert_eq!(name, "None"),
                    other => panic!("expected Ident pattern for `None`, got {other:?}"),
                }
            }
            other => panic!("expected Match, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Match), got {other:?}"),
    }
}

#[test]
fn match_unqualified_variant_err_wildcard() {
    // `Err _:` — variante desqualificada com wildcard como payload.
    let m = parse_src("match Result::Ok 42\n    Ok v: v\n    Err _: 0\n    otherwise: 0");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Match { arms, .. } => {
                assert_eq!(arms.len(), 3);
                // Err _ — unqualified variant with wildcard payload
                match &arms[1].pattern.as_ref().unwrap().node {
                    kata_ast::Pattern::Variant {
                        enum_name,
                        variant,
                        payload,
                    } => {
                        assert_eq!(enum_name, "");
                        assert_eq!(variant, "Err");
                        assert!(payload.is_some());
                        match &payload.as_ref().unwrap()[0].node {
                            kata_ast::Pattern::Wildcard => {}
                            other => panic!("expected Wildcard payload, got {other:?}"),
                        }
                    }
                    other => panic!("expected Variant pattern for `Err _`, got {other:?}"),
                }
            }
            other => panic!("expected Match, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Match), got {other:?}"),
    }
}

#[test]
fn match_binding_not_affected_by_unqualified_variant() {
    // `x:` em match sobre Int (não-enum) — deve continuar como Pattern::Ident (binding).
    // `x` não é seguido de algo que inicia pattern (`:` não é pattern start).
    let m = parse_src("match 5\n    x: x\n    otherwise: 0");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Match { arms, .. } => {
                assert_eq!(arms.len(), 2);
                match &arms[0].pattern.as_ref().unwrap().node {
                    kata_ast::Pattern::Ident(name) => assert_eq!(name, "x"),
                    other => panic!("expected Ident binding, got {other:?}"),
                }
            }
            other => panic!("expected Match, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Match), got {other:?}"),
    }
}
