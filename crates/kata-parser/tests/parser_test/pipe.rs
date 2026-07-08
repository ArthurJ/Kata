//! Pipeline `|>` parsing: basic pipe with hole, left-assoc chaining,
//! pipe without hole (inject as first arg).

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item};

#[test]
fn pipe_basic_with_hole() {
    // 5 |> + 10 _ → desugars to + 10 5, but parser produces Pipe
    let m = parse_src("5 |> + 10 _");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Pipe { lhs, rhs } => {
                assert_eq!(lhs.node, Expr::IntLit { text: "5".into() });
                // rhs should be Apply { +, [10, Hole] }
                match &rhs.node {
                    Expr::Apply { callee, args } => {
                        assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                        assert_eq!(args.len(), 2);
                        assert_eq!(args[0].node, Expr::IntLit { text: "10".into() });
                        assert_eq!(args[1].node, Expr::Hole);
                    }
                    other => panic!("expected Apply rhs, got {other:?}"),
                }
            }
            other => panic!("expected Pipe, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Pipe), got {other:?}"),
    }
}

#[test]
fn pipe_left_assoc_chained() {
    // 5 |> + 1 _ |> * 2 _ → left-assoc: (5 |> + 1 _) |> * 2 _
    let m = parse_src("5 |> + 1 _ |> * 2 _");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            // Outer pipe: lhs = (5 |> + 1 _), rhs = * 2 _
            Expr::Pipe { lhs, rhs } => {
                // lhs should be another Pipe
                match &lhs.node {
                    Expr::Pipe {
                        lhs: inner_lhs,
                        rhs: inner_rhs,
                    } => {
                        assert_eq!(inner_lhs.node, Expr::IntLit { text: "5".into() });
                        match &inner_rhs.node {
                            Expr::Apply { callee, .. } => {
                                assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                            }
                            other => panic!("expected Apply inner rhs, got {other:?}"),
                        }
                    }
                    other => panic!("expected nested Pipe, got {other:?}"),
                }
                // rhs should be Apply { *, [2, Hole] }
                match &rhs.node {
                    Expr::Apply { callee, args } => {
                        assert_eq!(callee.node, Expr::Ident { name: "*".into() });
                        assert_eq!(args.len(), 2);
                        assert_eq!(args[1].node, Expr::Hole);
                    }
                    other => panic!("expected Apply outer rhs, got {other:?}"),
                }
            }
            other => panic!("expected Pipe, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Pipe), got {other:?}"),
    }
}

#[test]
fn pipe_without_hole_injects_first_arg() {
    // 5 |> double → Pipe { lhs: 5, rhs: Ident("double") }
    // desugar will inject 5 as first arg
    let m = parse_src("5 |> double");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Pipe { lhs, rhs } => {
                assert_eq!(lhs.node, Expr::IntLit { text: "5".into() });
                assert_eq!(
                    rhs.node,
                    Expr::Ident {
                        name: "double".into()
                    }
                );
            }
            other => panic!("expected Pipe, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Pipe), got {other:?}"),
    }
}

#[test]
fn pipe_with_hole_in_first_position() {
    // 5 |> - _ 10 → Pipe { lhs: 5, rhs: Apply { -, [Hole, 10] } }
    let m = parse_src("5 |> - _ 10");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Pipe { lhs, rhs } => {
                assert_eq!(lhs.node, Expr::IntLit { text: "5".into() });
                match &rhs.node {
                    Expr::Apply { callee, args } => {
                        assert_eq!(callee.node, Expr::Ident { name: "-".into() });
                        assert_eq!(args.len(), 2);
                        assert_eq!(args[0].node, Expr::Hole);
                        assert_eq!(args[1].node, Expr::IntLit { text: "10".into() });
                    }
                    other => panic!("expected Apply rhs, got {other:?}"),
                }
            }
            other => panic!("expected Pipe, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Pipe), got {other:?}"),
    }
}
