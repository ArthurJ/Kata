//! Action parsing: bang call (ActionCall), action declaration (ActionDecl).

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item, TypeExpr};
use kata_lexer::lex;
use kata_parser::parse;

// ── ActionCall (bang call) ─────────────────────────────────────────

#[test]
fn bang_call_no_args() {
    let m = parse_src("greet!()");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ActionCall { callee, args } => {
                assert_eq!(callee, "greet");
                // `!()` produz Unit (tupla vazia).
                assert_eq!(args.node, Expr::Unit);
            }
            other => panic!("expected ActionCall, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn bang_call_one_arg() {
    // `echo!("hello")` — parênteses com 1 elemento (sem vírgula) = Grouping.
    let m = parse_src("echo!(\"hello\")");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ActionCall { callee, args } => {
                assert_eq!(callee, "echo");
                // `!("hello")` produz Grouping (parênteses de 1 elemento sem vírgula).
                match &args.node {
                    Expr::Grouping { inner } => {
                        assert_eq!(
                            inner.node,
                            Expr::TextLit {
                                text: "hello".into()
                            }
                        );
                    }
                    other => panic!("expected Grouping args, got {other:?}"),
                }
            }
            other => panic!("expected ActionCall, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn bang_call_one_arg_tuple() {
    // `echo!("hello",)` — parênteses com vírgula = Tuple de 1 elemento.
    let m = parse_src("echo!(\"hello\",)");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ActionCall { callee, args } => {
                assert_eq!(callee, "echo");
                match &args.node {
                    Expr::Tuple { elements } => {
                        assert_eq!(elements.len(), 1);
                        assert_eq!(
                            elements[0].node,
                            Expr::TextLit {
                                text: "hello".into()
                            }
                        );
                    }
                    other => panic!("expected Tuple args, got {other:?}"),
                }
            }
            other => panic!("expected ActionCall, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn bang_call_multi_args() {
    // `log!("msg", 42)` — vírgula separa elementos da tupla.
    let m = parse_src("log!(\"msg\", 42)");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ActionCall { callee, args } => {
                assert_eq!(callee, "log");
                match &args.node {
                    Expr::Tuple { elements } => {
                        assert_eq!(elements.len(), 2);
                        assert_eq!(elements[0].node, Expr::TextLit { text: "msg".into() });
                        assert_eq!(elements[1].node, Expr::IntLit { text: "42".into() });
                    }
                    other => panic!("expected Tuple args, got {other:?}"),
                }
            }
            other => panic!("expected ActionCall, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── ActionDecl ─────────────────────────────────────────────────────

#[test]
fn action_decl_no_params_no_ret() {
    let src = "action greet\n    echo!(\"hello\")";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl {
            name,
            params,
            ret,
            directives,
            body,
        } => {
            assert_eq!(name, "greet");
            assert!(params.is_empty());
            // Sem `->` = retorno Unit.
            assert_eq!(ret.node, TypeExpr::Unit);
            assert!(directives.is_empty());
            // Body tem 1 statement.
            assert_eq!(body.len(), 1);
            assert!(!body[0].has_semicolon);
            match &body[0].expr.node {
                Expr::ActionCall { callee, args } => {
                    assert_eq!(callee, "echo");
                    // `!("hello")` = Grouping (1 elemento sem vírgula).
                    match &args.node {
                        Expr::Grouping { inner } => match &inner.node {
                            Expr::TextLit { text } => assert_eq!(text, "hello"),
                            other => panic!("expected TextLit in Grouping, got {other:?}"),
                        },
                        other => panic!("expected Grouping args, got {other:?}"),
                    }
                }
                other => panic!("expected ActionCall in body, got {other:?}"),
            }
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_decl_with_params_and_ret() {
    let src = "action greet (Text) -> Unit\n    echo!(\"hello\")";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl {
            name,
            params,
            ret,
            directives,
            body,
        } => {
            assert_eq!(name, "greet");
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].node, TypeExpr::Named("Text".into()));
            // `-> Unit` parseia como TypeExpr::Named("Unit"), não TypeExpr::Unit (que é `()`).
            assert_eq!(ret.node, TypeExpr::Named("Unit".into()));
            assert!(directives.is_empty());
            assert_eq!(body.len(), 1);
            assert!(!body[0].has_semicolon);
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_decl_multi_statements() {
    let src = "action greet\n    echo!(\"hello\")\n    echo!(\"world\")";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { name, body, .. } => {
            assert_eq!(name, "greet");
            assert_eq!(body.len(), 2);
            // Primeiro statement: echo!("hello")
            assert!(
                matches!(&body[0].expr.node, Expr::ActionCall { callee, .. } if callee == "echo")
            );
            // Segundo statement: echo!("world")
            assert!(
                matches!(&body[1].expr.node, Expr::ActionCall { callee, .. } if callee == "echo")
            );
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_decl_then_entry_expr() {
    let src = "action greet\n    echo!(\"hello\")\ngreet!()";
    let m = parse_src(src);
    assert_eq!(m.items.len(), 2);
    assert!(matches!(m.items[0].node, Item::ActionDecl { .. }));
    assert!(matches!(
        &m.items[1].node,
        Item::EntryExpr(e) if matches!(e.node, Expr::ActionCall { .. })
    ));
}

#[test]
fn action_decl_semicolon_mark() {
    // Statement com `;` marca computação local (has_semicolon = true).
    let src = "action greet\n    echo!(\"hello\");\n    echo!(\"world\")";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => {
            assert_eq!(body.len(), 2);
            // Primeiro statement tem `;`.
            assert!(body[0].has_semicolon);
            // Segundo statement não tem `;` (retorno implícito).
            assert!(!body[1].has_semicolon);
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_decl_last_stmt_semicolon() {
    // Último statement com `;` → retorna Unit.
    let src = "action greet\n    echo!(\"hello\");";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => {
            assert_eq!(body.len(), 1);
            assert!(body[0].has_semicolon);
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_decl_tuple_return_type() {
    // Action com tipo de retorno tupla: `-> (Int, Int)`
    let src = "action make_pair -> (Int, Int)\n    (1, 2)";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl {
            name, params, ret, ..
        } => {
            assert_eq!(name, "make_pair");
            assert!(params.is_empty());
            match &ret.node {
                TypeExpr::Tuple(elements) => {
                    assert_eq!(elements.len(), 2);
                    assert_eq!(elements[0].node, TypeExpr::Named("Int".into()));
                    assert_eq!(elements[1].node, TypeExpr::Named("Int".into()));
                }
                other => panic!("expected TypeExpr::Tuple, got {other:?}"),
            }
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_decl_tuple_return_type_three() {
    // Tupla com 3 elementos: `-> (Int, Text, Boolean)`
    let src = "action triple -> (Int, Text, Boolean)\n    (1, \"hi\", True)";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { ret, .. } => match &ret.node {
            TypeExpr::Tuple(elements) => {
                assert_eq!(elements.len(), 3);
                assert_eq!(elements[0].node, TypeExpr::Named("Int".into()));
                assert_eq!(elements[1].node, TypeExpr::Named("Text".into()));
                assert_eq!(elements[2].node, TypeExpr::Named("Boolean".into()));
            }
            other => panic!("expected TypeExpr::Tuple, got {other:?}"),
        },
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

// ── loop, break, continue (Fase 4) ─────────────────────────────────

#[test]
fn loop_inside_action() {
    let src = "action contador\n    var i := 0\n    loop\n        i := + i 1\n        echo!(i)";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => {
            // body[0] = var i := 0
            // body[1] = loop { ... }
            assert_eq!(body.len(), 2);
            assert!(matches!(&body[0].expr.node, Expr::Var { name, .. } if name == "i"));
            match &body[1].expr.node {
                Expr::Loop { body: loop_body } => {
                    assert_eq!(loop_body.len(), 2);
                    // loop_body[0] = i := + i 1 (Reassign)
                    assert!(
                        matches!(&loop_body[0].node, Expr::Reassign { name, .. } if name == "i")
                    );
                    // loop_body[1] = echo!(i) (ActionCall)
                    assert!(
                        matches!(&loop_body[1].node, Expr::ActionCall { callee, .. } if callee == "echo")
                    );
                }
                other => panic!("expected Loop, got {other:?}"),
            }
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn break_continue_inside_loop_in_action() {
    let src = "action contador\n    var i := 0\n    loop\n        i := + i 1\n        echo!(i)\n        match > i 5\n            True: break\n            False: continue";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => {
            // body[1] = loop { ... }
            match &body[1].expr.node {
                Expr::Loop { body: loop_body } => {
                    // loop_body[2] = match > i 5 { True: break, False: continue }
                    match &loop_body[2].node {
                        Expr::Match { arms, .. } => {
                            assert_eq!(arms.len(), 2);
                            // arm 0: True: break
                            match &arms[0].body.node {
                                Expr::Break => {}
                                other => panic!("expected Break, got {other:?}"),
                            }
                            // arm 1: False: continue
                            match &arms[1].body.node {
                                Expr::Continue => {}
                                other => panic!("expected Continue, got {other:?}"),
                            }
                        }
                        other => panic!("expected Match, got {other:?}"),
                    }
                }
                other => panic!("expected Loop, got {other:?}"),
            }
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn loop_outside_action_errors() {
    let tokens = lex("loop\n    echo!(1)").unwrap();
    let result = parse(tokens);
    assert!(result.is_err(), "loop fora de Action deve produzir erro");
}

#[test]
fn break_outside_action_errors() {
    let tokens = lex("break").unwrap();
    let result = parse(tokens);
    assert!(result.is_err(), "break fora de Action deve produzir erro");
}

#[test]
fn continue_outside_action_errors() {
    let tokens = lex("continue").unwrap();
    let result = parse(tokens);
    assert!(
        result.is_err(),
        "continue fora de Action deve produzir erro"
    );
}
