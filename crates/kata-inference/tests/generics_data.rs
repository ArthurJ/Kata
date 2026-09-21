//! Testes E2E para generics paramétricos em `data`.
//!
//! Fase 3: smart constructor genérico, unify, bound check.

use kata_inference::{TypedModule, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::load_stdlib_for_tests;

// ── Helpers ───────────────────────────────────────────────────────

fn infer_src(src: &str) -> Result<TypedModule, kata_diagnostics::MiddleError> {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    infer_module(&module, &prelude)
}

// ── Testes ────────────────────────────────────────────────────────

/// `data Pair (first::T second::T) where T implements NUM`
/// Construtor aceita dois argumentos do mesmo tipo que implementa NUM.
/// `Pair 3 4` tipa como Pair::(Int, Int).
#[test]
fn generic_data_constructor_int() {
    let src = r#"
data Pair (first::T second::T) where T implements NUM
Pair 3 4
"#;
    let result = infer_src(src);
    assert!(result.is_ok(), "inferência deve succeed: {:?}", result.err());
}

/// `Pair 3.0 4.0` tipa como Pair::(Float, Float).
#[test]
fn generic_data_constructor_float() {
    let src = r#"
data Pair (first::T second::T) where T implements NUM
Pair 3.0 4.0
"#;
    let result = infer_src(src);
    assert!(result.is_ok(), "inferência deve succeed: {:?}", result.err());
}

/// `Pair "a" "b"` falha — Text não implementa NUM.
#[test]
fn generic_data_constructor_text_fails() {
    let src = r#"
data Pair (first::T second::T) where T implements NUM
Pair "a" "b"
"#;
    let result = infer_src(src);
    assert!(
        result.is_err(),
        "Text não implementa NUM — deve falhar"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("não implementa") || err.contains("NUM"),
        "erro deve mencionar NUM/não implementa, got: {err}"
    );
}

/// Type params livres (sem bound): `data Par (first::A second::B)`
/// aceita qualquer par de tipos.
#[test]
fn generic_data_free_params() {
    let src = r#"
data Par (first::A second::B)
Par 3 "hello"
"#;
    let result = infer_src(src);
    assert!(result.is_ok(), "inferência deve succeed: {:?}", result.err());
}

/// Type params livres com tipos iguais: `Par 3 4` também funciona.
#[test]
fn generic_data_free_params_same_type() {
    let src = r#"
data Par (first::A second::B)
Par 3 4
"#;
    let result = infer_src(src);
    assert!(result.is_ok(), "inferência deve succeed: {:?}", result.err());
}