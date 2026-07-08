use super::tokens_only;
use kata_ast::Token;
use kata_lexer::lex;

#[test]
fn string_double_quotes() {
    let tokens = lex("\"hello\"").unwrap();
    let expected = vec![Token::TextLit("hello".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_single_quotes() {
    let tokens = lex("'world'").unwrap();
    let expected = vec![Token::TextLit("world".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_escape_newline() {
    let tokens = lex("\"hello\\nworld\"").unwrap();
    let expected = vec![Token::TextLit("hello\nworld".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_escape_tab() {
    let tokens = lex("\"a\\tb\"").unwrap();
    let expected = vec![Token::TextLit("a\tb".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_escape_backslash() {
    let tokens = lex("\"a\\\\b\"").unwrap();
    let expected = vec![Token::TextLit("a\\b".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_escape_quote() {
    let tokens = lex("\"say \\\"hi\\\"\"").unwrap();
    let expected = vec![Token::TextLit("say \"hi\"".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_single_quote_escape() {
    let tokens = lex("'it\\'s'").unwrap();
    let expected = vec![Token::TextLit("it's".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_triple_quotes_raw() {
    let tokens = lex("\"\"\"raw\\ntext\"\"\"").unwrap();
    // Triplas são raw — \\n é literal, não escape
    let expected = vec![Token::TextLit("raw\\ntext".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_triple_quotes_multiline() {
    let tokens = lex("\"\"\"line1\nline2\"\"\"").unwrap();
    let expected = vec![Token::TextLit("line1\nline2".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_unknown_escape_preserved() {
    let tokens = lex("\"\\x\"").unwrap();
    let expected = vec![Token::TextLit("\\x".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}
