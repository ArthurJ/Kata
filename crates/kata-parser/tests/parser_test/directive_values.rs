//! Testes de parsing de diretivas com valores compostos (Fio 14 `@test`).
//!
//! D2: `DirectiveArg` é `Expr(Box<Spanned<Expr>>)` (posicional) ou
//! `Named { key, value: Box<Spanned<Expr>> }` (nomeado). Valores de diretiva
//! usam a mesma sintaxe que o resto da linguagem — tupla, variant, apply
//! posicional de construtor. Não há `DirectiveValue` separado.
//!
//! Compara `.node` (não `Spanned` inteiro) porque o span é real, não sintético.

use super::helpers::parse_src;
use kata_ast::{DirectiveArg, Expr, Item};

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
const ACTION_MIN: &str = "action foo\n    return 0";

#[test]
fn test_directive_short_form_string_only() {
    // @test("desc") — posicional, Expr::TextLit
    let m = parse_src(&format!("@test(\"descrição\")\n{ACTION_MIN}"));
    let d = first_test_directive(&m);
    assert_eq!(d.name, "test");
    assert_eq!(d.args.len(), 1);
    match &d.args[0] {
        DirectiveArg::Expr(e) => assert_eq!(e.node, Expr::TextLit { text: "descrição".into() }),
        other => panic!("expected Expr TextLit, got {other:?}"),
    }
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
            assert_eq!(value.node, Expr::TextLit { text: "soma".into() });
        }
        other => panic!("expected Named desc, got {other:?}"),
    }
    // args: (1, 2)
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            match &value.node {
                Expr::Tuple { elements } => {
                    assert_eq!(elements.len(), 2);
                    match &elements[0].node {
                        Expr::IntLit { text } => assert_eq!(text, "1"),
                        other => panic!("expected IntLit 1, got {other:?}"),
                    }
                    match &elements[1].node {
                        Expr::IntLit { text } => assert_eq!(text, "2"),
                        other => panic!("expected IntLit 2, got {other:?}"),
                    }
                }
                other => panic!("expected Tuple, got {other:?}"),
            }
        }
        other => panic!("expected Named args, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_variant_value_no_args() {
    // @test{desc: "...", args: (Result::Ok)} — variant unitária
    // O `(Result::Ok)` é Grouping { inner: VariantQual } — parênteses externos
    // viram Grouping, não Tuple (só 1 elemento, sem vírgula).
    let m = parse_src(&format!(
        "@test{{desc: \"ok sem args\", args: (Result::Ok)}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            match &value.node {
                Expr::Grouping { inner } => match &inner.node {
                    Expr::VariantQual { enum_name, variant } => {
                        assert_eq!(enum_name, "Result");
                        assert_eq!(variant, "Ok");
                    }
                    other => panic!("expected VariantQual inside Grouping, got {other:?}"),
                },
                other => panic!("expected Grouping, got {other:?}"),
            }
        }
        other => panic!("expected Named args, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_variant_apply_args() {
    // @test{desc: "...", args: (Result::Ok 42)} — variant com apply posicional
    // O `(Result::Ok 42)` é Grouping { inner: Apply(VariantQual, [42]) }
    let m = parse_src(&format!(
        "@test{{desc: \"ok com args\", args: (Result::Ok 42)}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            match &value.node {
                Expr::Grouping { inner } => match &inner.node {
                    Expr::Apply { callee, args } => {
                        match &callee.node {
                            Expr::VariantQual { enum_name, variant } => {
                                assert_eq!(enum_name, "Result");
                                assert_eq!(variant, "Ok");
                            }
                            other => panic!("expected VariantQual callee, got {other:?}"),
                        }
                        assert_eq!(args.len(), 1);
                        match &args[0].node {
                            Expr::IntLit { text } => assert_eq!(text, "42"),
                            other => panic!("expected IntLit 42, got {other:?}"),
                        }
                    }
                    other => panic!("expected Apply inside Grouping, got {other:?}"),
                },
                other => panic!("expected Grouping, got {other:?}"),
            }
        }
        other => panic!("expected Named args, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_struct_apply() {
    // @test{desc: "...", args: (Pessoa "João" 30)} — struct via apply posicional
    // O `(Pessoa "João" 30)` é Grouping { inner: Apply(Ident(Pessoa), [...]) }
    let m = parse_src(&format!(
        "@test{{desc: \"struct\", args: (Pessoa \"João\" 30)}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "args");
            match &value.node {
                Expr::Grouping { inner } => match &inner.node {
                    Expr::Apply { callee, args } => {
                        match &callee.node {
                            Expr::Ident { name } => assert_eq!(name, "Pessoa"),
                            other => panic!("expected Ident Pessoa, got {other:?}"),
                        }
                        assert_eq!(args.len(), 2);
                        match &args[0].node {
                            Expr::TextLit { text } => assert_eq!(text, "João"),
                            other => panic!("expected TextLit João, got {other:?}"),
                        }
                        match &args[1].node {
                            Expr::IntLit { text } => assert_eq!(text, "30"),
                            other => panic!("expected IntLit 30, got {other:?}"),
                        }
                    }
                    other => panic!("expected Apply inside Grouping, got {other:?}"),
                },
                other => panic!("expected Grouping, got {other:?}"),
            }
        }
        other => panic!("expected Named args, got {other:?}"),
    }
}

#[test]
fn test_directive_dict_with_expects_and_timeout() {
    // @test{desc: "...", expects: "Panic: msg", timeout: 5000}
    // `timeout` é keyword do lexer (Token::Timeout), aceito como key
    let m = parse_src(&format!(
        "@test{{desc: \"espera pânico\", expects: \"Panic: div\", timeout: 5000}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    assert_eq!(d.args.len(), 3);
    // desc
    match &d.args[0] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "desc");
            assert_eq!(value.node, Expr::TextLit { text: "espera pânico".into() });
        }
        other => panic!("expected Named desc, got {other:?}"),
    }
    // expects
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "expects");
            assert_eq!(value.node, Expr::TextLit { text: "Panic: div".into() });
        }
        other => panic!("expected Named expects, got {other:?}"),
    }
    // timeout
    match &d.args[2] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "timeout");
            assert_eq!(value.node, Expr::IntLit { text: "5000".into() });
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
            match &value.node {
                // `()` vazio é Unit em Kata, não Tuple vazia
                Expr::Unit => {}
                other => panic!("expected Unit (empty tuple), got {other:?}"),
            }
        }
        other => panic!("expected Named args, got {other:?}"),
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
            match &value.node {
                Expr::Tuple { elements } => {
                    assert_eq!(elements.len(), 2);
                    // Primeiro elemento: tupla (1, 2)
                    match &elements[0].node {
                        Expr::Tuple { elements: inner } => {
                            assert_eq!(inner.len(), 2);
                            match &inner[0].node {
                                Expr::IntLit { text } => assert_eq!(text, "1"),
                                other => panic!("expected IntLit 1, got {other:?}"),
                            }
                            match &inner[1].node {
                                Expr::IntLit { text } => assert_eq!(text, "2"),
                                other => panic!("expected IntLit 2, got {other:?}"),
                            }
                        }
                        other => panic!("expected inner Tuple, got {other:?}"),
                    }
                    // Segundo elemento: 3
                    match &elements[1].node {
                        Expr::IntLit { text } => assert_eq!(text, "3"),
                        other => panic!("expected IntLit 3, got {other:?}"),
                    }
                }
                other => panic!("expected Tuple, got {other:?}"),
            }
        }
        other => panic!("expected Named args, got {other:?}"),
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
    let m = parse_src(&format!(
        "@test{{desc: \"espera erro\", expects: \"CompileError: type mismatch\"}}\n{ACTION_MIN}"
    ));
    let d = first_test_directive(&m);
    assert_eq!(d.args.len(), 2);
    match &d.args[1] {
        DirectiveArg::Named { key, value } => {
            assert_eq!(key, "expects");
            assert_eq!(
                value.node,
                Expr::TextLit { text: "CompileError: type mismatch".into() }
            );
        }
        other => panic!("expected Named expects, got {other:?}"),
    }
}