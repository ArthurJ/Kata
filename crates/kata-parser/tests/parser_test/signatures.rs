//! Signature and directive parsing: sig declarations, @ffi, @associative,
//! stacked directives, named functions with lambda clauses.

use super::helpers::{first_item, parse_src};
use kata_ast::{DirectiveArg, Expr, Item, Span, Spanned, TypeExpr};

#[test]
fn sig_simple() {
    let m = parse_src("+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig {
            name,
            params,
            ret,
            directives,
            body,
        } => {
            assert_eq!(name, "+");
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
            assert_eq!(params[1].node, TypeExpr::Named("Int".into()));
            assert_eq!(ret.node, TypeExpr::Named("Int".into()));
            assert!(directives.is_empty());
            assert!(body.is_none());
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

// ── Directives ────────────────────────────────────────────────────

#[test]
fn directive_ffi_with_sig() {
    let m = parse_src("@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig {
            name, directives, ..
        } => {
            assert_eq!(name, "+");
            assert_eq!(directives.len(), 1);
            assert_eq!(directives[0].name, "ffi");
            assert_eq!(directives[0].args.len(), 1);
            match &directives[0].args[0] {
                DirectiveArg::Expr(e) => {
                    assert_eq!(
                        e.node,
                        Expr::TextLit {
                            text: "kata_rt_bi_add".into()
                        }
                    );
                }
                other => panic!("expected Expr arg, got {other:?}"),
            }
        }
        other => panic!("expected Sig with directive, got {other:?}"),
    }
}

#[test]
fn directive_associative_int() {
    let m = parse_src("@associative(0)\n+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig { directives, .. } => {
            assert_eq!(directives.len(), 1);
            assert_eq!(directives[0].name, "associative");
            assert_eq!(directives[0].args.len(), 1);
            match &directives[0].args[0] {
                DirectiveArg::Expr(e) => {
                    assert_eq!(e.node, Expr::IntLit { text: "0".into() });
                }
                other => panic!("expected Expr arg, got {other:?}"),
            }
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

#[test]
fn multiple_directives_stacked() {
    let m = parse_src("@ffi(\"kata_rt_bi_add\")\n@associative(0)\n+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig { directives, .. } => {
            assert_eq!(directives.len(), 2);
            assert_eq!(directives[0].name, "ffi");
            assert_eq!(directives[1].name, "associative");
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

// ── Named functions with lambda clauses ─────────────────────────

#[test]
fn sig_with_lambda_clauses() {
    let m = parse_src(
        "fat :: Int Int => Int\n    lambda 0 acc: acc\n    lambda n acc: fat (- n 1) (* n acc)",
    );
    let item = first_item(&m);
    match item {
        Item::Sig { name, body, .. } => {
            assert_eq!(name, "fat");
            let clauses = body.as_ref().expect("body should have clauses");
            assert_eq!(clauses.len(), 2);
            // First clause: lambda 0 acc: acc
            assert_eq!(clauses[0].node.patterns.len(), 2);
            match &clauses[0].node.patterns[0].node {
                kata_ast::Pattern::Literal(expr) => {
                    assert_eq!(expr.node, Expr::IntLit { text: "0".into() });
                }
                other => panic!("expected Literal pattern, got {other:?}"),
            }
            match &clauses[0].node.patterns[1].node {
                kata_ast::Pattern::Ident(name) => assert_eq!(name, "acc"),
                other => panic!("expected Ident pattern, got {other:?}"),
            }
            // Second clause: lambda n acc: fat (- n 1) (* n acc)
            assert_eq!(clauses[1].node.patterns.len(), 2);
            match &clauses[1].node.patterns[0].node {
                kata_ast::Pattern::Ident(name) => assert_eq!(name, "n"),
                other => panic!("expected Ident pattern, got {other:?}"),
            }
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

#[test]
fn sig_with_single_clause() {
    let m = parse_src("inc :: Int => Int\n    lambda x: + x 1");
    let item = first_item(&m);
    match item {
        Item::Sig { name, body, .. } => {
            assert_eq!(name, "inc");
            let clauses = body.as_ref().expect("body should have clauses");
            assert_eq!(clauses.len(), 1);
            assert_eq!(clauses[0].node.patterns.len(), 1);
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

#[test]
fn sig_without_body_still_works() {
    // FFI signatures (sem corpo) continuam funcionando
    let m = parse_src("@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig { name, body, .. } => {
            assert_eq!(name, "+");
            assert!(body.is_none());
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

#[test]
fn sig_with_lambda_unicode_clause() {
    let m = parse_src("inc :: Int => Int\n    λ x: + x 1");
    let item = first_item(&m);
    match item {
        Item::Sig { name, body, .. } => {
            assert_eq!(name, "inc");
            let clauses = body.as_ref().expect("body should have clauses");
            assert_eq!(clauses.len(), 1);
            assert_eq!(clauses[0].node.patterns.len(), 1);
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}
