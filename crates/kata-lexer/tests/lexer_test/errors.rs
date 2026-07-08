use kata_diagnostics::FrontendError;
use kata_lexer::lex;

#[test]
fn error_unterminated_string() {
    let result = lex("\"hello");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::UnterminatedString { .. } => {}
        e => panic!("esperado UnterminatedString, encontrado {:?}", e),
    }
}

#[test]
fn error_unterminated_single_quote_string() {
    let result = lex("'hello");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::UnterminatedString { .. } => {}
        e => panic!("esperado UnterminatedString, encontrado {:?}", e),
    }
}

#[test]
fn error_unterminated_triple_string() {
    let result = lex("\"\"\"hello");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::UnterminatedString { .. } => {}
        e => panic!("esperado UnterminatedString, encontrado {:?}", e),
    }
}
