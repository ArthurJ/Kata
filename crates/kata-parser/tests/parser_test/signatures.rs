//! Signature and directive parsing: sig declarations, @ffi, @associative,
//! stacked directives, named functions with lambda clauses.

use super::helpers::{first_item, parse_src};
use kata_ast::{DirectiveArg, Expr, Item, TypeExpr};

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
            ..
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
    let m =
        parse_src("fat :: Int Int => Int\nlambda 0 acc: acc\nlambda n acc: fat (- n 1) (* n acc)");
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
    let m = parse_src("inc :: Int => Int\nlambda x: + x 1");
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
    let m = parse_src("inc :: Int => Int\nλ x: + x 1");
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

// ── List shorthand [T] → List::T ──────────────────────────────────

#[test]
fn list_shorthand_single_param() {
    // `[Int]` desugara para `List::Int` (TypeExpr::ParamApp).
    let m = parse_src("quicksort :: [Int] => [Int]");
    let item = first_item(&m);
    match item {
        Item::Sig {
            name, params, ret, ..
        } => {
            assert_eq!(name, "quicksort");
            assert_eq!(params.len(), 1);
            // params[0] = [Int] → List::Int
            match &params[0].node {
                TypeExpr::ParamApp { name, params } => {
                    assert_eq!(name, "List");
                    assert_eq!(params.len(), 1);
                    assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
                }
                other => panic!("expected ParamApp for param, got {other:?}"),
            }
            // ret = [Int] → List::Int
            match &ret.node {
                TypeExpr::ParamApp { name, params } => {
                    assert_eq!(name, "List");
                    assert_eq!(params.len(), 1);
                    assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
                }
                other => panic!("expected ParamApp for ret, got {other:?}"),
            }
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

#[test]
fn list_shorthand_nested() {
    // `[[Int]]` desugara para `List::(List::Int)`.
    let m = parse_src("f :: [[Int]] => Int");
    let item = first_item(&m);
    match item {
        Item::Sig { params, .. } => {
            assert_eq!(params.len(), 1);
            match &params[0].node {
                TypeExpr::ParamApp { name, params } => {
                    assert_eq!(name, "List");
                    assert_eq!(params.len(), 1);
                    // Inner: List::Int
                    match &params[0].node {
                        TypeExpr::ParamApp { name, params } => {
                            assert_eq!(name, "List");
                            assert_eq!(params.len(), 1);
                            assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
                        }
                        other => panic!("expected nested ParamApp, got {other:?}"),
                    }
                }
                other => panic!("expected ParamApp, got {other:?}"),
            }
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

#[test]
fn list_shorthand_multi_param() {
    // `[A] [B]` — múltiplas listas como parâmetros separados.
    let m = parse_src("zip :: [A] [B] => Int");
    let item = first_item(&m);
    match item {
        Item::Sig { params, .. } => {
            assert_eq!(params.len(), 2);
            // params[0] = [A] → List::A
            match &params[0].node {
                TypeExpr::ParamApp { name, params } => {
                    assert_eq!(name, "List");
                    assert_eq!(params[0].node, TypeExpr::Named("A".into()));
                }
                other => panic!("expected ParamApp for param[0], got {other:?}"),
            }
            // params[1] = [B] → List::B
            match &params[1].node {
                TypeExpr::ParamApp { name, params } => {
                    assert_eq!(name, "List");
                    assert_eq!(params[0].node, TypeExpr::Named("B".into()));
                }
                other => panic!("expected ParamApp for param[1], got {other:?}"),
            }
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}
