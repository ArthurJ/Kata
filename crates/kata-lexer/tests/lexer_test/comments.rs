use super::tokens_only;
use kata_ast::Token;
use kata_lexer::lex;

#[test]
fn comment_line() {
    let tokens = lex("# this is a comment").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn comment_after_code() {
    let tokens = lex("42 # trailing comment").unwrap();
    let expected = vec![Token::IntLit("42".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn comment_between_statements() {
    let source = "let x := 1\n# comment line\nlet y := 2";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Let,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("1".into()),
        Token::StmtSep,
        Token::Let,
        Token::Ident("y".into()),
        Token::BindAssign,
        Token::IntLit("2".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn only_comments() {
    let tokens = lex("# comment 1\n# comment 2\n").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}
