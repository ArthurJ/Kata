//! Testes E2E de validação de casing.
//!
//! Testes positivos: nomes conformes passam no parser.
//! Testes negativos: nomes não-conformes produzem `FrontendError::InvalidCasing`.

use kata_diagnostics::FrontendError;
use kata_lexer::lex;
use kata_parser::parse;

fn parse_err(src: &str) -> FrontendError {
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap_err()
}

// ── Testes positivos: nomes conformes ─────────────────────────────

#[test]
fn data_decl_pascal_case_ok() {
    let src = "data Pessoa (nome::Text idade::Int)";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn enum_decl_pascal_case_ok() {
    let src = "enum Boolean\n    True\n    False";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn action_decl_snake_case_ok() {
    let src = "action soma (x::Int, y::Int) => Int\n    + x y";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn sig_snake_case_ok() {
    let src = "soma :: Int Int => Int";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn interface_all_caps_ok() {
    let src = "interface NUM\n    + :: NUM NUM => NUM";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn sig_symbol_name_ok() {
    // Nomes simbólicos (+, -, *) não são validados
    let src = "+ :: Int Int => Int";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn action_underscore_prefix_ok() {
    // _print começa com _ — é snake_case válido
    let src = "@ffi(\"kata_rt_print\")\naction _print (msg::Text) => Unit";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn refined_decl_pascal_case_ok() {
    let src = "data (Int, > _ 0) as PositiveInt";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn implements_decl_ok() {
    let src = "Text implements SHOW\n    to_text :: Text => Text";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

#[test]
fn struct_fields_snake_case_ok() {
    let src = "data Pessoa (nome::Text idade::Int altura::Float)";
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap();
}

// ── Testes negativos: nomes não-conformes ─────────────────────────

#[test]
fn data_decl_lowercase_fails() {
    let err = parse_err("data pessoa ()");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "pessoa"));
}

#[test]
fn enum_decl_lowercase_fails() {
    let err = parse_err("enum boolean\n    True\n    False");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "boolean"));
}

#[test]
fn enum_variant_lowercase_fails() {
    let err = parse_err("enum Boolean\n    True\n    false");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "false"));
}

#[test]
fn action_decl_pascal_case_fails() {
    let err = parse_err("action Soma (x::Int) => Int\n    x");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "Soma"));
}

#[test]
fn sig_pascal_case_fails() {
    let err = parse_err("Soma :: Int Int => Int");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "Soma"));
}

#[test]
fn interface_lowercase_fails() {
    let err = parse_err("interface num\n    + :: num num => num");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "num"));
}

#[test]
fn interface_pascal_case_fails() {
    let err = parse_err("interface Num\n    + :: Num Num => Num");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "Num"));
}

#[test]
fn struct_field_pascal_case_fails() {
    let err = parse_err("data Pessoa (Nome::Text)");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "Nome"));
}

#[test]
fn action_param_pascal_case_fails() {
    let err = parse_err("action soma (X::Int) => Int\n    X");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "X"));
}

#[test]
fn refined_decl_lowercase_fails() {
    let err = parse_err("data (Int, > _ 0) as positiveInt");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "positiveInt"));
}

#[test]
fn implements_type_lowercase_fails() {
    let err = parse_err("texto implements SHOW\n    to_text :: texto => texto");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "texto"));
}

#[test]
fn implements_iface_lowercase_fails() {
    let err = parse_err("Text implements show\n    to_text :: Text => Text");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "show"));
}

#[test]
fn implements_method_pascal_case_fails() {
    let src = "Text implements SHOW\n    ToText :: Text => Text";
    let err = parse_err(src);
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "ToText"));
}

#[test]
fn alias_lowercase_target_fails() {
    let err = parse_err("alias float as Altura");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "float"));
}

#[test]
fn alias_lowercase_new_name_fails() {
    let err = parse_err("alias Float as altura");
    assert!(matches!(err, FrontendError::InvalidCasing { ref name, .. } if name == "altura"));
}

// ── Verificação da mensagem de erro ───────────────────────────────

#[test]
fn invalid_casing_message_content() {
    let err = parse_err("data pessoa ()");
    match err {
        FrontendError::InvalidCasing {
            name,
            expected_casing,
            found_casing,
            ..
        } => {
            assert_eq!(name, "pessoa");
            assert_eq!(expected_casing, "PascalCase");
            assert_eq!(found_casing, "snake_case");
        }
        other => panic!("expected InvalidCasing, got {other:?}"),
    }
}