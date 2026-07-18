//! CSP parser tests: channel send (`!>`), channel recv (`<!`),
//! select with timeout, fork! as ActionCall.

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item, SelectArm};

// ── Token lexing: !> and <! ────────────────────────────────────────

#[test]
fn lex_send_arrow() {
    // `tx !> 42` — !> é um único token SendArrow
    let m = parse_src("tx !> 42");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ChannelSend { channel, value } => {
                assert_eq!(channel.node, Expr::Ident { name: "tx".into() });
                assert_eq!(value.node, Expr::IntLit { text: "42".into() });
            }
            other => panic!("expected ChannelSend, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn lex_recv_arrow() {
    // `rx <! msg` — <! é um único token RecvArrow
    let m = parse_src("rx <! msg");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ChannelRecv { channel, bind_name } => {
                assert_eq!(channel.node, Expr::Ident { name: "rx".into() });
                assert_eq!(bind_name, "msg");
            }
            other => panic!("expected ChannelRecv, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── <! still allows < as operator ──────────────────────────────────

#[test]
fn less_than_still_ident() {
    // `<` sem `!` logo após continua sendo Ident (operador de comparação)
    let m = parse_src("< x 0");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(callee.node, Expr::Ident { name: "<".into() });
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Apply, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── ! still works as Bang (Action call) ────────────────────────────

#[test]
fn bang_still_works() {
    // `echo!("hi")` — ! sem > continua sendo Bang
    let m = parse_src("echo!(\"hi\")");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ActionCall { callee, .. } => {
                assert_eq!(callee, "echo");
            }
            other => panic!("expected ActionCall, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── Channel send with complex value ────────────────────────────────

#[test]
fn channel_send_complex_value() {
    // `tx !> + 1 2` — send aplica greedy depois de !>
    let m = parse_src("tx !> + 1 2");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ChannelSend { channel, value } => {
                assert_eq!(channel.node, Expr::Ident { name: "tx".into() });
                match &value.node {
                    Expr::Apply { callee, args } => {
                        assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                        assert_eq!(args.len(), 2);
                    }
                    other => panic!("expected Apply value, got {other:?}"),
                }
            }
            other => panic!("expected ChannelSend, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── fork! is ActionCall ────────────────────────────────────────────

#[test]
fn fork_is_action_call() {
    // `fork!(worker, (arg1, arg2))` — parseado como ActionCall builtin
    let m = parse_src("fork!(worker, (arg1, arg2))");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ActionCall { callee, args } => {
                assert_eq!(callee, "fork");
                // args é a tupla (worker, (arg1, arg2))
                match &args.node {
                    Expr::Tuple { elements } => {
                        assert_eq!(elements.len(), 2);
                    }
                    other => panic!("expected Tuple args, got {other:?}"),
                }
            }
            other => panic!("expected ActionCall, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── channel! is ActionCall ─────────────────────────────────────────

#[test]
fn channel_create_is_action_call() {
    let m = parse_src("channel!()");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ActionCall { callee, args } => {
                assert_eq!(callee, "channel");
                assert_eq!(args.node, Expr::Unit);
            }
            other => panic!("expected ActionCall, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── select inside Action ───────────────────────────────────────────

#[test]
fn select_basic() {
    let src = "action exemplo\n    select\n        rx <! msg: echo!(msg)\n        rx2 <! item: echo!(item)";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => {
            assert_eq!(body.len(), 1);
            match &body[0].expr.node {
                Expr::Select {
                    arms,
                    timeout_ms,
                    timeout_body,
                } => {
                    assert_eq!(arms.len(), 2);
                    assert!(timeout_ms.is_none());
                    assert!(timeout_body.is_none());
                    // Primeiro braço: rx <! msg: echo!(msg)
                    check_select_arm(&arms[0], "rx", "msg", "echo");
                    // Segundo braço: rx2 <! item: echo!(item)
                    check_select_arm(&arms[1], "rx2", "item", "echo");
                }
                other => panic!("expected Select, got {other:?}"),
            }
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn select_with_timeout() {
    let src = "action exemplo\n    select\n        rx <! msg: echo!(msg)\n        timeout 5000: echo!(\"timeout\")";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => {
            assert_eq!(body.len(), 1);
            match &body[0].expr.node {
                Expr::Select {
                    arms,
                    timeout_ms,
                    timeout_body,
                } => {
                    assert_eq!(arms.len(), 1);
                    check_select_arm(&arms[0], "rx", "msg", "echo");
                    assert!(timeout_ms.is_some());
                    assert!(timeout_body.is_some());
                }
                other => panic!("expected Select, got {other:?}"),
            }
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

fn check_select_arm(
    arm: &SelectArm,
    expected_channel: &str,
    expected_bind: &str,
    expected_callee: &str,
) {
    assert_eq!(
        arm.channel.node,
        Expr::Ident {
            name: expected_channel.into()
        }
    );
    assert_eq!(arm.bind_name, expected_bind);
    match &arm.body.node {
        Expr::ActionCall { callee, .. } => {
            assert_eq!(callee, expected_callee);
        }
        other => panic!("expected ActionCall body, got {other:?}"),
    }
}

// ── select outside Action is error ─────────────────────────────────

#[test]
fn select_outside_action_is_error() {
    use kata_lexer::lex;
    use kata_parser::parse;
    let src = "select\n    rx <! msg: echo!(msg)";
    let tokens = lex(src).unwrap();
    let result = parse(tokens);
    assert!(
        result.is_err(),
        "select outside Action should be a parse error"
    );
}

// ── Channel send/recv in let binding ───────────────────────────────

#[test]
fn channel_send_in_let() {
    // `let x := tx !> 42` — let value é ChannelSend
    let m = parse_src("let x := tx !> 42");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Let { name, value } => {
                assert_eq!(name, "x");
                assert!(matches!(value.node, Expr::ChannelSend { .. }));
            }
            other => panic!("expected Let, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── Left-associativity of !> ───────────────────────────────────────

#[test]
fn send_arrow_left_assoc() {
    // `a !> b !> c` = `(a !> b) !> c` — left-associative
    // O primeiro `!>` produz ChannelSend(a, b).
    // O segundo `!>` produz ChannelSend(ChannelSend(a, b), c).
    let m = parse_src("a !> b !> c");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::ChannelSend { channel, value } => {
                // channel = (a !> b)
                match &channel.node {
                    Expr::ChannelSend {
                        channel: inner_ch,
                        value: inner_val,
                    } => {
                        assert_eq!(inner_ch.node, Expr::Ident { name: "a".into() });
                        assert_eq!(inner_val.node, Expr::Ident { name: "b".into() });
                    }
                    other => panic!("expected nested ChannelSend, got {other:?}"),
                }
                // value = c
                assert_eq!(value.node, Expr::Ident { name: "c".into() });
            }
            other => panic!("expected ChannelSend, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}
