//! Guards and with block parsing: guard clauses inside lambda body,
//! `otherwise` fallback, `with` bindings.

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, GuardClause, Item};

// ── Guards in anonymous lambda ───────────────────────────────────

#[test]
fn lambda_anon_with_guards() {
    let src = "lambda x:\n    > x 0: x\n    otherwise: - 0 x";
    let m = parse_src(src);
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
                // guards should have 2 clauses: > x 0 and otherwise
                assert_eq!(guards.len(), 2);
                // First guard: > x 0 → condition = (> x 0), body = x
                match &guards[0] {
                    GuardClause {
                        condition: Some(cond),
                        body: guard_body,
                    } => {
                        match &cond.node {
                            Expr::Apply { callee, args } => {
                                assert_eq!(callee.node, Expr::Ident { name: ">".into() });
                                assert_eq!(args.len(), 2);
                            }
                            other => panic!("expected Apply condition, got {other:?}"),
                        }
                        assert_eq!(guard_body.node, Expr::Ident { name: "x".into() });
                    }
                    other => panic!("expected guard with condition, got {other:?}"),
                }
                // Second guard: otherwise → condition = None, body = - 0 x
                match &guards[1] {
                    GuardClause {
                        condition: None,
                        body: guard_body,
                    } => match &guard_body.node {
                        Expr::Apply { callee, args } => {
                            assert_eq!(callee.node, Expr::Ident { name: "-".into() });
                            assert_eq!(args.len(), 2);
                        }
                        other => panic!("expected Apply body, got {other:?}"),
                    },
                    other => panic!("expected otherwise guard, got {other:?}"),
                }
                // body should be present (fallback or first body)
                assert!(with_bindings.is_empty());
                // body exists but is not used when guards are present
                let _ = body;
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_guard_single_condition() {
    // Single guard with condition, no otherwise
    let src = "lambda x:\n    > x 0: x";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { guards, .. } => {
                assert_eq!(guards.len(), 1);
                assert!(guards[0].condition.is_some());
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

// ── Guards in named function clauses ─────────────────────────────

#[test]
fn sig_clause_with_guards() {
    let src = "abs :: Int => Int\n    lambda x:\n        > x 0: x\n        otherwise: - 0 x";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::Sig { name, body, .. } => {
            assert_eq!(name, "abs");
            let clauses = body.as_ref().expect("body should have clauses");
            assert_eq!(clauses.len(), 1);
            let clause = &clauses[0].node;
            assert_eq!(clause.patterns.len(), 1);
            assert_eq!(clause.guards.len(), 2);
            assert!(clause.guards[0].condition.is_some());
            assert!(clause.guards[1].condition.is_none()); // otherwise
            assert!(clause.with_bindings.is_empty());
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

// ── with block ────────────────────────────────────────────────────

#[test]
fn lambda_with_block() {
    let src = "lambda x:\n    > doubled 10: \"grande\"\n    otherwise: \"pequeno\"\n    with\n        doubled := * x 2";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda {
                guards,
                with_bindings,
                ..
            } => {
                assert_eq!(guards.len(), 2);
                assert!(guards[0].condition.is_some());
                assert!(guards[1].condition.is_none());
                // with block has 1 binding: doubled := * x 2
                assert_eq!(with_bindings.len(), 1);
                assert_eq!(with_bindings[0].name, "doubled");
                match &with_bindings[0].value.node {
                    Expr::Apply { callee, args } => {
                        assert_eq!(callee.node, Expr::Ident { name: "*".into() });
                        assert_eq!(args.len(), 2);
                    }
                    other => panic!("expected Apply in with binding, got {other:?}"),
                }
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_with_block_multiple_bindings() {
    let src = "lambda x:\n    otherwise: y\n    with\n        y := + x 1\n        z := * y 2";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { with_bindings, .. } => {
                assert_eq!(with_bindings.len(), 2);
                assert_eq!(with_bindings[0].name, "y");
                assert_eq!(with_bindings[1].name, "z");
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

// ── Body direto sem guards + with (sem otherwise) ───────────────

#[test]
fn lambda_body_direct_with_block_no_guards() {
    // Lambda com body direto (sem guards) + with block.
    // `otherwise:` é dispensável quando não há guards competindo.
    let src = "lambda [pivo:resto]:\n    + (quicksort menores) [pivo : (quicksort maiores)]\n    with\n        menores := filter (< _ pivo) resto\n        maiores := filter (>= _ pivo) resto";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda {
                guards,
                body,
                with_bindings,
                ..
            } => {
                // Sem guards — body direto
                assert!(guards.is_empty(), "deveria não ter guards");
                // Body é a expressão direta (Apply de +)
                match &body.node {
                    Expr::Apply { callee, .. } => {
                        assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                    }
                    other => panic!("expected Apply body, got {other:?}"),
                }
                // with block tem 2 bindings
                assert_eq!(with_bindings.len(), 2);
                assert_eq!(with_bindings[0].name, "menores");
                assert_eq!(with_bindings[1].name, "maiores");
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn sig_clause_body_direct_no_guards_with_block() {
    // Cláusula de função nomeada com body direto (sem guards) + with.
    let src = "quicksort :: [Int] => [Int]\n    lambda []: []\n    lambda [pivo:resto]:\n        + (quicksort menores) [pivo : (quicksort maiores)]\n        with\n            menores := filter (< _ pivo) resto\n            maiores := filter (>= _ pivo) resto";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::Sig { name, body, .. } => {
            assert_eq!(name, "quicksort");
            let clauses = body.as_ref().expect("body should have clauses");
            assert_eq!(clauses.len(), 2);
            // Segunda cláusula: body direto sem guards
            let clause = &clauses[1].node;
            assert!(clause.guards.is_empty(), "segunda cláusula não deveria ter guards");
            assert_eq!(clause.with_bindings.len(), 2);
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}
