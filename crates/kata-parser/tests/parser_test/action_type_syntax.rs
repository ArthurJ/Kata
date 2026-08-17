//! Action type syntax: `Action(Params) -> Ret`.

use super::helpers::{first_item, parse_src};
use kata_ast::{Item, TypeExpr};

#[test]
fn action_type_no_params() {
    let m = parse_src("action f => Action() -> Unit\n    Unit");
    let item = first_item(&m);
    match item {
        Item::ActionDecl { ret, .. } => match &ret.node {
            TypeExpr::ActionType { params, ret } => {
                assert_eq!(params.len(), 0);
                assert_eq!(ret.node, TypeExpr::Named("Unit".into()));
            }
            other => panic!("expected ActionType, got {other:?}"),
        },
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_type_one_param() {
    let m = parse_src("action f => Action(Int) -> Unit\n    Unit");
    let item = first_item(&m);
    match item {
        Item::ActionDecl { ret, .. } => match &ret.node {
            TypeExpr::ActionType { params, ret } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
                assert_eq!(ret.node, TypeExpr::Named("Unit".into()));
            }
            other => panic!("expected ActionType, got {other:?}"),
        },
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_type_multi_params() {
    let m = parse_src("action f => Action(Int, Text) -> Boolean\n    True");
    let item = first_item(&m);
    match item {
        Item::ActionDecl { ret, .. } => match &ret.node {
            TypeExpr::ActionType { params, ret } => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
                assert_eq!(params[1].node, TypeExpr::Named("Text".into()));
                assert_eq!(ret.node, TypeExpr::Named("Boolean".into()));
            }
            other => panic!("expected ActionType, got {other:?}"),
        },
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

#[test]
fn action_type_in_action_param_signature() {
    // Action with a param whose type is an Action type:
    // `action f (g :: Action(Int) -> Unit) => Unit`
    let src = "action f (g :: Action(Int) -> Unit) => Unit\n    g!(1)";
    let m = parse_src(src);
    let item = first_item(&m);
    match item {
        Item::ActionDecl {
            params,
            param_names,
            ret,
            ..
        } => {
            // One param: g :: Action(Int) -> Unit
            assert_eq!(params.len(), 1);
            assert_eq!(param_names.len(), 1);
            assert_eq!(param_names[0], Some("g".to_string()));
            match &params[0].node {
                TypeExpr::ActionType { params, ret } => {
                    assert_eq!(params.len(), 1);
                    assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
                    assert_eq!(ret.node, TypeExpr::Named("Unit".into()));
                }
                other => panic!("expected ActionType param, got {other:?}"),
            }
            // Return type: Unit (named, not `()`)
            assert_eq!(ret.node, TypeExpr::Named("Unit".into()));
        }
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}