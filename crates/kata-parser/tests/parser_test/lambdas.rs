//! Lambda and hole parsing: anonymous lambdas, patterns, holes in apply.

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item};

#[test]
fn lambda_anon_simple() {
    let m = parse_src("lambda x: + x 1");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda {
                patterns,
                body,
                guards,
                with_bindings,
            } => {
                assert_eq!(patterns.len(), 1);
                assert_eq!(patterns[0].node, kata_ast::Pattern::Ident("x".into()));
                // body é `+ x 1` (Apply)
                match &body.node {
                    Expr::Apply { callee, args } => {
                        assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                        assert_eq!(args.len(), 2);
                    }
                    other => panic!("expected Apply body, got {other:?}"),
                }
                assert!(guards.is_empty());
                assert!(with_bindings.is_empty());
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_multi_pattern() {
    let m = parse_src("lambda x y: + x y");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 2);
                assert_eq!(patterns[0].node, kata_ast::Pattern::Ident("x".into()));
                assert_eq!(patterns[1].node, kata_ast::Pattern::Ident("y".into()));
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_unicode() {
    let m = parse_src("λ n: + n 1");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                assert_eq!(patterns[0].node, kata_ast::Pattern::Ident("n".into()));
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_wildcard_pattern() {
    let m = parse_src("lambda _: 42");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                assert_eq!(patterns[0].node, kata_ast::Pattern::Wildcard);
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_literal_pattern() {
    let m = parse_src("lambda 0: 1");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                match &patterns[0].node {
                    kata_ast::Pattern::Literal(expr) => {
                        assert_eq!(expr.node, Expr::IntLit { text: "0".into() });
                    }
                    other => panic!("expected Literal pattern, got {other:?}"),
                }
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_variant_pattern() {
    let m = parse_src("lambda Boolean::True: 1");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                match &patterns[0].node {
                    kata_ast::Pattern::Variant { enum_name, variant } => {
                        assert_eq!(enum_name, "Boolean");
                        assert_eq!(variant, "True");
                    }
                    other => panic!("expected Variant pattern, got {other:?}"),
                }
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_tuple_pattern() {
    let m = parse_src("lambda (a, b): a");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                match &patterns[0].node {
                    kata_ast::Pattern::Tuple(elements) => {
                        assert_eq!(elements.len(), 2);
                        assert_eq!(elements[0].node, kata_ast::Pattern::Ident("a".into()));
                        assert_eq!(elements[1].node, kata_ast::Pattern::Ident("b".into()));
                    }
                    other => panic!("expected Tuple pattern, got {other:?}"),
                }
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

// ── Hole (`_`) em posição de argumento ───────────────────────────

#[test]
fn hole_in_apply_arg() {
    let m = parse_src("+ 10 _");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].node, Expr::IntLit { text: "10".into() });
                assert_eq!(args[1].node, Expr::Hole);
            }
            other => panic!("expected Apply with Hole, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Apply), got {other:?}"),
    }
}

#[test]
fn hole_multiple() {
    let m = parse_src("+ _ _");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].node, Expr::Hole);
                assert_eq!(args[1].node, Expr::Hole);
            }
            other => panic!("expected Apply with 2 Holes, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Apply), got {other:?}"),
    }
}
