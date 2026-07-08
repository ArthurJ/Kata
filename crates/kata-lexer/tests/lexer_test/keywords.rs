use super::tokens_only;
use kata_ast::Token;
use kata_lexer::lex;

#[test]
fn keyword_let() {
    let tokens = lex("let").unwrap();
    let expected = vec![Token::Let, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_var() {
    let tokens = lex("var").unwrap();
    let expected = vec![Token::Var, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_data() {
    let tokens = lex("data").unwrap();
    let expected = vec![Token::Data, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_enum() {
    let tokens = lex("enum").unwrap();
    let expected = vec![Token::Enum, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_lambda() {
    let tokens = lex("lambda").unwrap();
    let expected = vec![Token::Lambda, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_import() {
    let tokens = lex("import").unwrap();
    let expected = vec![Token::Import, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_export() {
    let tokens = lex("export").unwrap();
    let expected = vec![Token::Export, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_as() {
    let tokens = lex("as").unwrap();
    let expected = vec![Token::As, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_interface() {
    let tokens = lex("interface").unwrap();
    let expected = vec![Token::Interface, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_implements() {
    let tokens = lex("implements").unwrap();
    let expected = vec![Token::Implements, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_with() {
    let tokens = lex("with").unwrap();
    let expected = vec![Token::With, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_match() {
    let tokens = lex("match").unwrap();
    let expected = vec![Token::Match, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_return() {
    let tokens = lex("return").unwrap();
    let expected = vec![Token::Return, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_otherwise() {
    let tokens = lex("otherwise").unwrap();
    let expected = vec![Token::Otherwise, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_alias() {
    let tokens = lex("alias").unwrap();
    let expected = vec![Token::Alias, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_action() {
    let tokens = lex("action").unwrap();
    let expected = vec![Token::Action, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn lambda_unicode() {
    let tokens = lex("λ").unwrap();
    let expected = vec![Token::Lambda, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn identifier_not_keyword() {
    let tokens = lex("myVar").unwrap();
    let expected = vec![Token::Ident("myVar".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn identifier_camelcase() {
    let tokens = lex("MyType").unwrap();
    // CamelCase é identificador normal em Kata5 (sem UpperIdent)
    let expected = vec![Token::Ident("MyType".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}
