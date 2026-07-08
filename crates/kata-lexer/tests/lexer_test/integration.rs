use super::tokens_only;
use kata_ast::Token;
use kata_lexer::lex;

#[test]
fn simple_arithmetic_prefix() {
    let tokens = lex("+ 1 2").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn let_binding() {
    let tokens = lex("let x := 42").unwrap();
    let expected = vec![
        Token::Let,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("42".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn ffi_directive_and_signature() {
    let source = "@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::At,
        Token::Ident("ffi".into()),
        Token::LParen,
        Token::TextLit("kata_rt_bi_add".into()),
        Token::RParen,
        Token::StmtSep,
        Token::Ident("+".into()),
        Token::DoubleColon,
        Token::Ident("Int".into()),
        Token::Ident("Int".into()),
        Token::FatArrow,
        Token::Ident("Int".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn data_decl_opaque() {
    let source = "data Int ()";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Data,
        Token::Ident("Int".into()),
        Token::LParen,
        Token::RParen,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn complex_expression() {
    // `+ 1 (* 2 3)` — aplicação prefixa com parênteses
    let tokens = lex("+ 1 (* 2 3)").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::LParen,
        Token::Ident("*".into()),
        Token::IntLit("2".into()),
        Token::IntLit("3".into()),
        Token::RParen,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn empty_source() {
    let tokens = lex("").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn only_whitespace() {
    let tokens = lex("   \n  \n  ").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}
