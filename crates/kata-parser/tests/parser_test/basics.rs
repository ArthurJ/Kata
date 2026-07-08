//! Basic expression parsing: apply, let, ascription, tuples, grouping, unit,
//! variant qual, text literal, ident, declarations, multi-item, greedy application.

use super::helpers::{parse_src, first_item};
use kata_ast::{DirectiveArg, Expr, Item, TypeExpr};

#[test]
fn apply_plus_1_2() {
    let m = parse_src("+ 1 2");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].node, Expr::IntLit { text: "1".into() });
                assert_eq!(args[1].node, Expr::IntLit { text: "2".into() });
            }
            other => panic!("expected Apply, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn let_binding() {
    let m = parse_src("let x := 42");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Let { name, value } => {
                assert_eq!(name, "x");
                assert_eq!(value.node, Expr::IntLit { text: "42".into() });
            }
            other => panic!("expected Let, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn type_ascription_rational() {
    let m = parse_src("3.14::Rational");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::TypeAscription { expr, ty } => {
                assert_eq!(
                    expr.node,
                    Expr::FloatLit {
                        text: "3.14".into()
                    }
                );
                assert_eq!(ty.node, TypeExpr::Named("Rational".into()));
            }
            other => panic!("expected TypeAscription, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn tuple_three_elements() {
    let m = parse_src("(1, 2, 3)");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Tuple { elements } => {
                assert_eq!(elements.len(), 3);
                assert_eq!(elements[0].node, Expr::IntLit { text: "1".into() });
                assert_eq!(elements[1].node, Expr::IntLit { text: "2".into() });
                assert_eq!(elements[2].node, Expr::IntLit { text: "3".into() });
            }
            other => panic!("expected Tuple, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn tuple_trailing_comma() {
    let m = parse_src("(42,)");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Tuple { elements } => {
                assert_eq!(elements.len(), 1);
                assert_eq!(elements[0].node, Expr::IntLit { text: "42".into() });
            }
            other => panic!("expected Tuple, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn grouping_single() {
    let m = parse_src("(42)");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Grouping { inner } => {
                assert_eq!(inner.node, Expr::IntLit { text: "42".into() });
            }
            other => panic!("expected Grouping, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn unit_literal() {
    let m = parse_src("()");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => assert_eq!(e.node, Expr::Unit),
        other => panic!("expected EntryExpr(Unit), got {other:?}"),
    }
}

#[test]
fn variant_qual() {
    let m = parse_src("Boolean::True");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::VariantQual { enum_name, variant } => {
                assert_eq!(enum_name, "Boolean");
                assert_eq!(variant, "True");
            }
            other => panic!("expected VariantQual, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── Declarations ──────────────────────────────────────────────────

#[test]
fn data_decl_empty() {
    let m = parse_src("data Int ()");
    let item = first_item(&m);
    match item {
        Item::DataDecl {
            name,
            fields,
            directives,
        } => {
            assert_eq!(name, "Int");
            assert!(fields.is_empty());
            assert!(directives.is_empty());
        }
        other => panic!("expected DataDecl, got {other:?}"),
    }
}

#[test]
fn enum_decl_variants() {
    let m = parse_src("enum Boolean\n    True\n    False");
    let item = first_item(&m);
    match item {
        Item::EnumDecl {
            name,
            variants,
            directives,
        } => {
            assert_eq!(name, "Boolean");
            assert_eq!(variants.len(), 2);
            assert_eq!(variants[0].name, "True");
            assert_eq!(variants[1].name, "False");
            assert!(directives.is_empty());
        }
        other => panic!("expected EnumDecl, got {other:?}"),
    }
}

// ── Multi-item module ────────────────────────────────────────────

#[test]
fn multiple_items() {
    let src = "data Int ()\nenum Boolean\n    True\n    False\n+ 1 2";
    let m = parse_src(src);
    assert_eq!(m.items.len(), 3);
    assert!(matches!(m.items[0].node, Item::DataDecl { .. }));
    assert!(matches!(m.items[1].node, Item::EnumDecl { .. }));
    assert!(matches!(m.items[2].node, Item::EntryExpr(_)));
}

// ── Greedy application ────────────────────────────────────────────

#[test]
fn greedy_application_three_args() {
    let m = parse_src("f a b c");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(callee.node, Expr::Ident { name: "f".into() });
                assert_eq!(args.len(), 3);
                assert_eq!(args[0].node, Expr::Ident { name: "a".into() });
                assert_eq!(args[1].node, Expr::Ident { name: "b".into() });
                assert_eq!(args[2].node, Expr::Ident { name: "c".into() });
            }
            other => panic!("expected Apply, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── Text literals ────────────────────────────────────────────────

#[test]
fn text_literal() {
    let m = parse_src("\"hello\"");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => assert_eq!(
            e.node,
            Expr::TextLit {
                text: "hello".into()
            }
        ),
        other => panic!("expected EntryExpr(TextLit), got {other:?}"),
    }
}

// ── Identifier as expression ─────────────────────────────────────

#[test]
fn ident_alone() {
    let m = parse_src("x");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => assert_eq!(e.node, Expr::Ident { name: "x".into() }),
        other => panic!("expected EntryExpr(Ident), got {other:?}"),
    }
}