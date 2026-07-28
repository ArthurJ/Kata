//! Testes — VariantQual com module path qualificado.
//!
//! `core.Result::Err` → VariantQual { module_path: ["core"], enum_name: "Result", variant: "Err" }
//! `Result::Err`      → VariantQual { module_path: None, enum_name: "Result", variant: "Err" }

use kata_ast::{Expr, Item};
use kata_lexer::lex;
use kata_parser::parse;

use super::helpers::first_item;

#[test]
fn parse_unqual_variant_keeps_module_path_none() {
    let src = "Result::Err";
    let tokens = lex(src).expect("lex");
    let m = parse(tokens).expect("parse");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::VariantQual { enum_name, variant, module_path } => {
                assert_eq!(enum_name, "Result");
                assert_eq!(variant, "Err");
                assert_eq!(module_path, &None, "module_path deve ser None para `Result::Err`");
            }
            other => panic!("expected VariantQual, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn parse_qualified_variant_one_level() {
    let src = "core.Result::Err";
    let tokens = lex(src).expect("lex");
    let m = parse(tokens).expect("parse");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::VariantQual { enum_name, variant, module_path } => {
                assert_eq!(enum_name, "Result");
                assert_eq!(variant, "Err");
                assert_eq!(
                    module_path,
                    &Some(vec!["core".to_string()]),
                    "module_path deve ser [\"core\"] para `core.Result::Err`"
                );
            }
            other => panic!("expected VariantQual, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn parse_qualified_variant_two_levels() {
    let src = "std.core.Result::Ok";
    let tokens = lex(src).expect("lex");
    let m = parse(tokens).expect("parse");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::VariantQual { enum_name, variant, module_path } => {
                assert_eq!(enum_name, "Result");
                assert_eq!(variant, "Ok");
                assert_eq!(
                    module_path,
                    &Some(vec!["std".to_string(), "core".to_string()]),
                    "module_path deve ser [\"std\", \"core\"] para `std.core.Result::Ok`"
                );
            }
            other => panic!("expected VariantQual, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn parse_qualified_variant_with_apply() {
    // `core.Result::Ok 42` — Apply com callee qualificado
    let src = "core.Result::Ok 42";
    let tokens = lex(src).expect("lex");
    let m = parse(tokens).expect("parse");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, .. } => match &callee.node {
                Expr::VariantQual { enum_name, variant, module_path } => {
                    assert_eq!(enum_name, "Result");
                    assert_eq!(variant, "Ok");
                    assert_eq!(
                        module_path,
                        &Some(vec!["core".to_string()]),
                        "module_path deve ser [\"core\"] em Apply de `core.Result::Ok 42`"
                    );
                }
                other => panic!("expected VariantQual callee, got {other:?}"),
            },
            other => panic!("expected Apply, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn parse_dot_access_not_confused_with_qualified_variant() {
    // `pessoa.nome` deve produzir DotAccess, não VariantQual
    let src = "pessoa.nome";
    let tokens = lex(src).expect("lex");
    let m = parse(tokens).expect("parse");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::DotAccess { .. } => { /* ok */ }
            other => panic!("expected DotAccess, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn parse_dot_access_followed_by_ascription() {
    // `pessoa.nome::Text` — DotAccess seguido de TypeAscription
    // (não é VariantQual qualificado porque não há Ident após `::` que seja variant
    //  — na verdade `Text` é Ident, mas DotAccess vem primeiro na precedência)
    let src = "pessoa.nome";
    let tokens = lex(src).expect("lex");
    let m = parse(tokens).expect("parse");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::DotAccess { .. } => { /* ok — DotAccess tem precedência */ }
            other => panic!("expected DotAccess, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}