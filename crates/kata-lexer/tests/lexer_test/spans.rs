use kata_ast::Token;
use kata_lexer::lex;

#[test]
fn span_of_first_token() {
    let tokens = lex("+ 1 2").unwrap();
    let first = &tokens[0];
    assert_eq!(first.token, Token::Ident("+".into()));
    assert_eq!(first.span.offset, 0);
    assert_eq!(first.span.line, 1);
    assert_eq!(first.span.col, 1);
    assert_eq!(first.span.len, 1);
}

#[test]
fn span_of_second_token() {
    let tokens = lex("+ 1 2").unwrap();
    let second = &tokens[1];
    assert_eq!(second.token, Token::IntLit("1".into()));
    assert_eq!(second.span.offset, 2);
    assert_eq!(second.span.line, 1);
    assert_eq!(second.span.col, 3);
    assert_eq!(second.span.len, 1);
}

#[test]
fn span_multiline_second_line() {
    let source = "+ 1 2\n+ 3 4";
    let tokens = lex(source).unwrap();
    // Token `+` na segunda linha (após StmtSep)
    let plus_idx = tokens
        .iter()
        .position(|t| t.token == Token::Ident("+".into()) && t.span.line == 2)
        .expect("encontrou + na linha 2");
    let plus = &tokens[plus_idx];
    assert_eq!(plus.span.line, 2);
    assert_eq!(plus.span.col, 1);
    assert_eq!(plus.span.offset, 6); // "+ 1 2\n" = 6 bytes
    assert_eq!(plus.span.len, 1);
}
