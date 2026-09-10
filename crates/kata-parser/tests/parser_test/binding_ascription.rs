//! Parser tests for binding ascription: `let x::Type := expr` and `var x::Type := expr`.
//!
//! PRD: docs/PRDs/PRD-binding-ascription-widening.md

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item, TypeExpr};

// ── let with ascription (inside action) ──────────────────────

#[test]
fn let_ascription_int() {
    let src = "action main\n    let x::Int := 42\n    echo!(x)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::Let { name, ty, value } => {
                assert_eq!(name, "x");
                assert!(ty.is_some(), "ty deve ser Some");
                match &ty.as_ref().unwrap().node {
                    TypeExpr::Named(n) => assert_eq!(n, "Int"),
                    other => panic!("esperado TypeExpr::Named, got {other:?}"),
                }
                assert!(matches!(&value.node, Expr::IntLit { text } if text == "42"));
            }
            other => panic!("esperado Let, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}

#[test]
fn let_ascription_text() {
    let src = "action main\n    let y::Text := \"hello\"\n    echo!(y)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::Let { name, ty, .. } => {
                assert_eq!(name, "y");
                match &ty.as_ref().unwrap().node {
                    TypeExpr::Named(n) => assert_eq!(n, "Text"),
                    other => panic!("esperado TypeExpr::Named(\"Text\"), got {other:?}"),
                }
            }
            other => panic!("esperado Let, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}

#[test]
fn let_ascription_interface() {
    let src = "action main\n    let z::NUM := 0\n    echo!(z)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::Let { name, ty, .. } => {
                assert_eq!(name, "z");
                match &ty.as_ref().unwrap().node {
                    TypeExpr::Named(n) => assert_eq!(n, "NUM"),
                    other => panic!("esperado TypeExpr::Named(\"NUM\"), got {other:?}"),
                }
            }
            other => panic!("esperado Let, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}

#[test]
fn let_without_ascription_has_none_ty() {
    let src = "action main\n    let x := 42\n    echo!(x)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::Let { name, ty, .. } => {
                assert_eq!(name, "x");
                assert!(ty.is_none(), "ty deve ser None sem ascription");
            }
            other => panic!("esperado Let, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}

// ── var with ascription ──────────────────────────────────────

#[test]
fn var_ascription_int() {
    let src = "action main\n    var x::Int := 0\n    echo!(x)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::Var { name, ty, .. } => {
                assert_eq!(name, "x");
                assert!(ty.is_some(), "ty deve ser Some");
                match &ty.as_ref().unwrap().node {
                    TypeExpr::Named(n) => assert_eq!(n, "Int"),
                    other => panic!("esperado TypeExpr::Named, got {other:?}"),
                }
            }
            other => panic!("esperado Var, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}

#[test]
fn var_ascription_interface() {
    let src = "action main\n    var z::NUM := 0\n    echo!(z)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::Var { name, ty, .. } => {
                assert_eq!(name, "z");
                match &ty.as_ref().unwrap().node {
                    TypeExpr::Named(n) => assert_eq!(n, "NUM"),
                    other => panic!("esperado TypeExpr::Named(\"NUM\"), got {other:?}"),
                }
            }
            other => panic!("esperado Var, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}

#[test]
fn var_without_ascription_has_none_ty() {
    let src = "action main\n    var x := 0\n    echo!(x)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::Var { name, ty, .. } => {
                assert_eq!(name, "x");
                assert!(ty.is_none(), "ty deve ser None sem ascription");
            }
            other => panic!("esperado Var, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}

// ── ascription with complex types ────────────────────────────

#[test]
fn let_ascription_result_type() {
    let src = "action main\n    let x::Result::(Int, Text) := Ok 42\n    echo!(x)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::Let { name, ty, .. } => {
                assert_eq!(name, "x");
                assert!(ty.is_some());
            }
            other => panic!("esperado Let, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}

// ── destructuring does NOT support ascription ────────────────

#[test]
fn destructuring_ascription_is_not_let() {
    let src = "action main\n    let (x, y) := (1, 2)\n    echo!(x)\nmain!()";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl { body, .. } => match &body[0].expr.node {
            Expr::LetDestruct { names, .. } => {
                assert_eq!(names.len(), 2);
                assert_eq!(names[0], "x");
                assert_eq!(names[1], "y");
            }
            other => panic!("esperado LetDestruct, got {other:?}"),
        },
        other => panic!("esperado ActionDecl, got {other:?}"),
    }
}
