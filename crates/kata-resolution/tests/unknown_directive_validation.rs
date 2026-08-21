//! Resolution: validação de diretivas desconhecidas.
//!
//! Diretivas fora da lista válida do contexto (Sig, Action, Implements method,
//! Data) produzem `ResolveError::UnknownDirective`. Garante que typos como
//! `@tset` ou `@fffi` não passem silenciosamente.

use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolveError, resolve};

fn resolve_src_err(src: &str) -> Vec<ResolveError> {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    resolve(&module).unwrap_err()
}

#[test]
fn unknown_directive_in_sig_rejected() {
    // @tset é typo de @test — deveria estar em Action, não em Sig
    let errors = resolve_src_err("@tset(\"desc\")\n+ :: Int Int => Int");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            ResolveError::UnknownDirective {
                name,
                context: "sig",
                ..
            } if name == "tset"
        )),
        "esperava UnknownDirective para `tset` em sig, obtive: {errors:?}"
    );
}

#[test]
fn unknown_directive_in_action_rejected() {
    // @builtin não é válida em Action (só em Implements)
    let errors = resolve_src_err("@builtin(\"foo\")\naction bar\n    return 0");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            ResolveError::UnknownDirective {
                name,
                context: "action",
                ..
            } if name == "builtin"
        )),
        "esperava UnknownDirective para `builtin` em action, obtive: {errors:?}"
    );
}

#[test]
fn test_directive_in_action_accepted() {
    // @test é válida em Action — resolve deve succeed
    let tokens = lex("@test(\"desc\")\naction foo => Int\n    return 42").unwrap();
    let module = parse(tokens).unwrap();
    resolve(&module).expect("@test em Action deve ser aceita");
}

#[test]
fn test_directive_in_sig_rejected() {
    // @test NÃO é válida em Sig — só em Action
    let errors = resolve_src_err("@test(\"desc\")\n+ :: Int Int => Int");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            ResolveError::UnknownDirective {
                name,
                context: "sig",
                ..
            } if name == "test"
        )),
        "esperava UnknownDirective para `test` em sig, obtive: {errors:?}"
    );
}

#[test]
fn ffi_directive_in_sig_accepted() {
    let tokens = lex("@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int").unwrap();
    let module = parse(tokens).unwrap();
    resolve(&module).expect("@ffi em Sig deve ser aceita");
}

#[test]
fn builtin_directive_in_sig_accepted() {
    // @builtin é válida em Sig (mesmo que só faça sentido em Implements — o
    // parser de Sig não rejeita, e o resolution não processa, mas aceita)
    let tokens = lex("@builtin(\"foo\")\n+ :: Int Int => Int").unwrap();
    let module = parse(tokens).unwrap();
    resolve(&module).expect("@builtin em Sig deve ser aceita");
}

#[test]
fn unknown_directive_in_implements_method_rejected() {
    // @tset é typo — não é válida em implements
    let src = "Int implements NUM\n    @tset(\"desc\")\n    + :: Int Int => Int";
    let errors = resolve_src_err(src);
    assert!(
        errors.iter().any(|e| matches!(
            e,
            ResolveError::UnknownDirective {
                name,
                context: "implements method",
                ..
            } if name == "tset"
        )),
        "esperava UnknownDirective para `tset` em implements method, obtive: {errors:?}"
    );
}

#[test]
fn unknown_directive_in_data_rejected() {
    // @comutative é typo de @commutative — não é válida em data
    let errors = resolve_src_err("@comutative\ndata Pessoa (nome::Text)");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            ResolveError::UnknownDirective {
                name,
                context: "data",
                ..
            } if name == "comutative"
        )),
        "esperava UnknownDirective para `comutative` em data, obtive: {errors:?}"
    );
}

#[test]
fn ffi_directive_in_data_accepted() {
    let tokens = lex("@ffi(\"i64\")\ndata Int64 (val::Int)").unwrap();
    let module = parse(tokens).unwrap();
    resolve(&module).expect("@ffi em data deve ser aceita");
}

#[test]
fn commutative_in_implements_accepted() {
    let src = "Int implements NUM\n    @commutative\n    + :: Int Int => Int";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    resolve(&module).expect("@commutative em implements deve ser aceita");
}
