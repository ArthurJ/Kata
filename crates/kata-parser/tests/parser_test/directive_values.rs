//! Testes de parsing de diretivas com valores compostos (Fio 14 `@test`).
//!
//! `DirectiveValue` suporta tupla e variant além de `Str`/`Int`. Structs
//! NÃO são suportadas como `DirectiveValue::Struct` — structs em Kata são
//! construídas via apply posicional, parseado pelo `parse_atom` existente.

use super::helpers::parse_src;
use kata_ast::{DirectiveArg, DirectiveValue, Item};

/// Extrai a primeira diretiva `test` do primeiro item do módulo.
fn first_test_directive(m: &kata_ast::Module) -> &kata_ast::Directive {
    let item = &m.items.first().expect("at least one item").node;
    match item {
        Item::ActionDecl { directives, .. } => directives
            .iter()
            .find(|d| d.name == "test")
            .expect("expected @test directive"),
        other => panic!("expected ActionDecl, got {other:?}"),
    }
}

/// Action mínima para testar parsing de diretiva — sem params, sem body útil.
/// O parser exige INDENT (4 espaços) após `action nome`.
const ACTION_MIN: &str = "action foo\n    return 0";

#[test]
fn test_directive_short_form_string_only() {
    // @test("desc") — tupla de 1 string (forma curta)
    let m = parse_src(&format!("@test(\"descrição\")\n{ACTION_MIN}"));
    let d = first_test_directive(&m);
    assert_eq!(d.name, "test");
    assert_eq!(d.args, vec![DirectiveArg::Str("descrição".into())]);
}

#[test]
fn test_directive_dict_with_tuple_value() {
    // @test{desc: "...", args: (1, 2)} — dict com tupla como valor
    let m = parse_src(&format!(
        "@test{{desc: \"soma\", args: (1, 2)}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    assert_eq!(d.args.len(), 2);
    // desc
    match &d.args[0] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "desc");
            assert_eq!(value, &DirectiveValue::Str("soma".into()));
        }
        other => panic!("expected Named desc, got {other:?}"),
    }
    // args: (1, 2) — tupla com dois Int
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            assert_eq!(
                value,
                &DirectiveValue::Tuple(vec![
                    DirectiveValue::Int(1),
                    DirectiveValue::Int(2),
                ])
            );
        }
        other => panic!("expected Named args with Tuple, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_variant_value_no_args() {
    // @test{desc: "...", args: (Result::Ok)} — variant unitária
    let m = parse_src(&format!(
        "@test{{desc: \"ok sem args\", args: (Result::Ok)}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            assert_eq!(
                value,
                &DirectiveValue::Tuple(vec![DirectiveValue::Variant(
                    "Result::Ok".into(),
                    vec![]
                )])
            );
        }
        other => panic!("expected Named args with Variant, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_variant_value_with_paren_args() {
    // @test{desc: "...", args: (Result::Ok(42))} — variant com args entre parênteses
    let m = parse_src(&format!(
        "@test{{desc: \"ok com args\", args: (Result::Ok(42))}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            assert_eq!(
                value,
                &DirectiveValue::Tuple(vec![DirectiveValue::Variant(
                    "Result::Ok".into(),
                    vec![DirectiveValue::Int(42)]
                )])
            );
        }
        other => panic!("expected Named args with Variant+args, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_expects_and_timeout() {
    // @test{desc: "...", expects: "Panic: msg", timeout: 5000} — dict sem args
    // `timeout` é keyword do lexer (Token::Timeout), não Ident — o parser de
    // diretivas precisa aceitar ambas como key.
    let m = parse_src(&format!(
        "@test{{desc: \"espera pânico\", expects: \"Panic: div\", timeout: 5000}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    assert_eq!(d.args.len(), 3);
    // desc
    match &d.args[0] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "desc");
            assert_eq!(value, &DirectiveValue::Str("espera pânico".into()));
        }
        other => panic!("expected Named desc, got {other:?}"),
    }
    // expects
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "expects");
            assert_eq!(value, &DirectiveValue::Str("Panic: div".into()));
        }
        other => panic!("expected Named expects, got {other:?}"),
    }
    // timeout
    match &d.args[2] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "timeout");
            assert_eq!(value, &DirectiveValue::Int(5000));
        }
        other => panic!("expected Named timeout, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_empty_tuple_args() {
    // @test{desc: "...", args: ()} — tupla vazia (action sem args)
    let m = parse_src(&format!(
        "@test{{desc: \"sem args\", args: ()}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            assert_eq!(value, &DirectiveValue::Tuple(vec![]));
        }
        other => panic!("expected Named args with empty Tuple, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_nested_tuple() {
    // @test{desc: "...", args: ((1, 2), 3)} — tupla aninhada
    let m = parse_src(&format!(
        "@test{{desc: \"aninhada\", args: ((1, 2), 3)}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            assert_eq!(
                value,
                &DirectiveValue::Tuple(vec![
                    DirectiveValue::Tuple(vec![
                        DirectiveValue::Int(1),
                        DirectiveValue::Int(2),
                    ]),
                    DirectiveValue::Int(3),
                ])
            );
        }
        other => panic!("expected Named args with nested Tuple, got {other:?}"),
    }
}

#[test]
fn existing_directives_still_parse() {
    // @ffi, @commutative continuam funcionando — não regrediu
    let m = parse_src("@ffi(\"kata_rt_bi_add\")\n@commutative(0)\n+ :: Int Int => Int");
    match &m.items[0].node {
        Item::Sig { directives, .. } => {
            assert_eq!(directives.len(), 2);
            assert_eq!(directives[0].name, "ffi");
            assert_eq!(directives[1].name, "commutative");
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

#[test]
fn test_directive_negative_form() {
    // @test{desc: "...", expects: "CompileError: msg"} — teste negativo
    // (a action alvo fica em arquivo isolado no runner; o parser só valida
    // a sintaxe da diretiva)
    let m = parse_src(&format!(
        "@test{{desc: \"espera erro\", expects: \"CompileError: type mismatch\"}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    assert_eq!(d.args.len(), 2);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "expects");
            assert_eq!(
                value,
                &DirectiveValue::Str("CompileError: type mismatch".into())
            );
        }
        other => panic!("expected Named expects CompileError, got {other:?}"),
    }
}