use super::tokens_only;
use kata_ast::Token;
use kata_lexer::lex;

// ── Números Int ────────────────────────────────────────────────

#[test]
fn int_decimal() {
    let tokens = lex("42").unwrap();
    let expected = vec![Token::IntLit("42".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_hex() {
    let tokens = lex("0xFF").unwrap();
    let expected = vec![Token::IntLit("0xFF".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_octal() {
    let tokens = lex("0o77").unwrap();
    let expected = vec![Token::IntLit("0o77".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_binary() {
    let tokens = lex("0b1010").unwrap();
    let expected = vec![Token::IntLit("0b1010".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_explicit_decimal() {
    let tokens = lex("0d255").unwrap();
    let expected = vec![Token::IntLit("0d255".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_underscore_separator() {
    let tokens = lex("1_000").unwrap();
    // underscore é descartado léxicamente
    let expected = vec![Token::IntLit("1000".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_zero() {
    let tokens = lex("0").unwrap();
    let expected = vec![Token::IntLit("0".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Float ──────────────────────────────────────────────────────

#[test]
fn float_decimal() {
    let tokens = lex("3.14").unwrap();
    let expected = vec![Token::FloatLit("3.14".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_scientific() {
    let tokens = lex("1.5e10").unwrap();
    let expected = vec![Token::FloatLit("1.5e10".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_scientific_negative_exp() {
    let tokens = lex("1.5E-10").unwrap();
    let expected = vec![Token::FloatLit("1.5E-10".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_scientific_no_point() {
    let tokens = lex("1e10").unwrap();
    let expected = vec![Token::FloatLit("1e10".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_with_underscore() {
    let tokens = lex("1_000.5").unwrap();
    let expected = vec![Token::FloatLit("1000.5".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_leading_dot() {
    // `+ .6` — espaço antes do `.` → float normalizado para `0.6`
    let tokens = lex("+ .6").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::FloatLit("0.6".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_leading_dot_start_of_input() {
    // `.6` no início do input → float (sem token anterior)
    let tokens = lex(".6").unwrap();
    let expected = vec![Token::FloatLit("0.6".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_leading_dot_scientific() {
    // `+ .5e10` — espaço antes do `.` → float normalizado para `0.5e10`
    let tokens = lex("+ .5e10").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::FloatLit("0.5e10".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn dot_access_not_float() {
    // `tpl.0` sem espaço → Dot + IntLit (dot-access de tupla)
    let tokens = lex("tpl.0").unwrap();
    let expected = vec![
        Token::Ident("tpl".into()),
        Token::Dot,
        Token::IntLit("0".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Números com sinal ──────────────────────────────────────────

#[test]
fn signed_number_no_space() {
    // `+1` sem espaço = número positivo
    let tokens = lex("+1").unwrap();
    let expected = vec![Token::IntLit("+1".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn negative_number_no_space() {
    let tokens = lex("-42").unwrap();
    let expected = vec![Token::IntLit("-42".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn error_invalid_number_no_digits_after_prefix() {
    let result = lex("0x");
    assert!(result.is_err());
    match result.unwrap_err() {
        kata_diagnostics::FrontendError::InvalidNumber { .. } => {}
        e => panic!("esperado InvalidNumber, encontrado {:?}", e),
    }
}

#[test]
fn error_invalid_float_exp_no_digits() {
    let result = lex("1.5e");
    assert!(result.is_err());
    match result.unwrap_err() {
        kata_diagnostics::FrontendError::InvalidNumber { .. } => {}
        e => panic!("esperado InvalidNumber, encontrado {:?}", e),
    }
}
