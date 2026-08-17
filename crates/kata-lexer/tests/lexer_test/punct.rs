use super::tokens_only;
use kata_ast::Token;
use kata_lexer::lex;

// ── Pontuação e operadores ─────────────────────────────────────

#[test]
fn punct_bind_assign() {
    let tokens = lex(":=").unwrap();
    let expected = vec![Token::BindAssign, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_double_colon() {
    let tokens = lex("::").unwrap();
    let expected = vec![Token::DoubleColon, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_fat_arrow() {
    let tokens = lex("=>").unwrap();
    let expected = vec![Token::FatArrow, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_thin_arrow() {
    let tokens = lex("->").unwrap();
    let expected = vec![Token::ThinArrow, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_pipe() {
    let tokens = lex("|").unwrap();
    let expected = vec![Token::Pipe, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_pipe_forward() {
    let tokens = lex("|>").unwrap();
    let expected = vec![Token::PipeForward, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_question() {
    let tokens = lex("?").unwrap();
    let expected = vec![Token::Question, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_bang() {
    let tokens = lex("!").unwrap();
    let expected = vec![Token::Bang, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_parens() {
    let tokens = lex("()").unwrap();
    let expected = vec![Token::LParen, Token::RParen, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_brackets() {
    let tokens = lex("[]").unwrap();
    let expected = vec![Token::LBracket, Token::RBracket, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_braces() {
    let tokens = lex("{}").unwrap();
    let expected = vec![Token::LBrace, Token::RBrace, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_comma() {
    let tokens = lex(",").unwrap();
    let expected = vec![Token::Comma, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_dot() {
    let tokens = lex(".").unwrap();
    let expected = vec![Token::Dot, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_semicolon() {
    let tokens = lex(";").unwrap();
    let expected = vec![Token::Semicolon, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_colon() {
    let tokens = lex(":").unwrap();
    let expected = vec![Token::Colon, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_at() {
    let tokens = lex("@").unwrap();
    let expected = vec![Token::At, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn equal_alone_is_ident() {
    let tokens = lex("=").unwrap();
    let expected = vec![Token::Ident("=".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn minus_alone_is_ident() {
    let tokens = lex("-").unwrap();
    let expected = vec![Token::Ident("-".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Operadores como identificadores ────────────────────────────

#[test]
fn operators_as_identifiers() {
    let tokens = lex("+ - * / < > =").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::Ident("-".into()),
        Token::Ident("*".into()),
        Token::Ident("/".into()),
        Token::Ident("<".into()),
        Token::Ident(">".into()),
        Token::Ident("=".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}
