use super::tokens_only;
use kata_ast::Token;
use kata_lexer::lex;

// ═══════════════════════════════════════════════════════════════════
// b"..." — aspas duplas (já funcionava)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn bytes_double_quotes() {
    let tokens = lex("b\"Hello\"").unwrap();
    let expected = vec![Token::BytesLit(b"Hello".to_vec()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn bytes_double_quotes_empty() {
    let tokens = lex("b\"\"").unwrap();
    let expected = vec![Token::BytesLit(vec![]), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn bytes_double_quotes_hex_escape() {
    let tokens = lex("b\"\\x00\\xFF\"").unwrap();
    let expected = vec![Token::BytesLit(vec![0x00, 0xFF]), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ═══════════════════════════════════════════════════════════════════
// b'...' — aspas simples (nova paridade com Text)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn bytes_single_quotes() {
    let tokens = lex("b'Hello'").unwrap();
    let expected = vec![Token::BytesLit(b"Hello".to_vec()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn bytes_single_quotes_empty() {
    let tokens = lex("b''").unwrap();
    let expected = vec![Token::BytesLit(vec![]), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn bytes_single_quotes_hex_escape() {
    let tokens = lex("b'\\x00\\xFF'").unwrap();
    let expected = vec![Token::BytesLit(vec![0x00, 0xFF]), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn bytes_single_quotes_escape_quote() {
    let tokens = lex("b'it\\'s'").unwrap();
    let expected = vec![Token::BytesLit(b"it's".to_vec()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn bytes_single_quotes_escape_newline() {
    let tokens = lex("b'a\\nb'").unwrap();
    let expected = vec![Token::BytesLit(b"a\nb".to_vec()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ═══════════════════════════════════════════════════════════════════
// Equivalência — b"..." e b'...' produzem o mesmo BytesLit
// ═══════════════════════════════════════════════════════════════════

#[test]
fn bytes_double_and_single_quotes_equivalent() {
    let double = lex("b\"Hello\\x00\"").unwrap();
    let single = lex("b'Hello\\x00'").unwrap();
    assert_eq!(tokens_only(&double), tokens_only(&single));
}

// ═══════════════════════════════════════════════════════════════════
// Erros — string não terminada
// ═══════════════════════════════════════════════════════════════════

#[test]
fn bytes_single_quotes_unterminated() {
    let result = lex("b'Hello");
    assert!(result.is_err(), "b'Hello sem fechamento deve erro");
}

#[test]
fn bytes_double_quotes_unterminated() {
    let result = lex("b\"Hello");
    assert!(result.is_err(), "b\"Hello sem fechamento deve erro");
}
