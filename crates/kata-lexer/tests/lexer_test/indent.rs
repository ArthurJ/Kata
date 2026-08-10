use super::tokens_only;
use kata_ast::Token;
use kata_lexer::lex;

#[test]
fn indent_simple_block() {
    let source = "constant x := 1\n    + 1 2";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Constant,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("1".into()),
        Token::Indent,
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::Dedent,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn indent_no_change_no_stmtsep_at_eof() {
    // Ultima linha não gera StmtSep antes de EOF
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
fn stmtsep_between_statements() {
    let source = "+ 1 2\n+ 3 4";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::StmtSep,
        Token::Ident("+".into()),
        Token::IntLit("3".into()),
        Token::IntLit("4".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn indent_dedent_block() {
    let source = "constant x := 1\n    + 1 2\n+ 3 4";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Constant,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("1".into()),
        Token::Indent,
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::Dedent,
        Token::Ident("+".into()),
        Token::IntLit("3".into()),
        Token::IntLit("4".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn blank_lines_ignored() {
    let source = "+ 1 2\n\n\n+ 3 4";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::StmtSep,
        Token::Ident("+".into()),
        Token::IntLit("3".into()),
        Token::IntLit("4".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn newline_in_parens_no_stmtsep() {
    let source = "(+ 1\n2)";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::LParen,
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::RParen,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn enum_block_indent() {
    let source = "enum Boolean\n    True\n    False";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Enum,
        Token::Ident("Boolean".into()),
        Token::Indent,
        Token::Ident("True".into()),
        Token::StmtSep,
        Token::Ident("False".into()),
        Token::Dedent,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn nested_indent_levels() {
    let source = "a\n    b\n        c\n    d";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Ident("a".into()),
        Token::Indent,
        Token::Ident("b".into()),
        Token::Indent,
        Token::Ident("c".into()),
        Token::Dedent,
        Token::Ident("d".into()),
        Token::Dedent,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}
