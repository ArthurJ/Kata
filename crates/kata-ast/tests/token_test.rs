use kata_ast::Token;

#[test]
fn int_lit_is_literal() {
    assert!(Token::IntLit("42".into()).is_literal());
}

#[test]
fn float_lit_is_literal() {
    assert!(Token::FloatLit("3.14".into()).is_literal());
}

#[test]
fn text_lit_is_literal() {
    assert!(Token::TextLit("hello".into()).is_literal());
}

#[test]
fn ident_is_not_literal() {
    assert!(!Token::Ident("x".into()).is_literal());
}

#[test]
fn let_is_keyword() {
    assert!(Token::Let.is_keyword());
}

#[test]
fn data_is_keyword() {
    assert!(Token::Data.is_keyword());
}

#[test]
fn enum_is_keyword() {
    assert!(Token::Enum.is_keyword());
}

#[test]
fn ident_plus_is_not_keyword() {
    assert!(!Token::Ident("+".into()).is_keyword());
}

#[test]
fn token_display_int_lit() {
    assert_eq!(format!("{}", Token::IntLit("42".into())), "42");
}

#[test]
fn token_display_ident_plus() {
    assert_eq!(format!("{}", Token::Ident("+".into())), "+");
}

#[test]
fn token_display_let() {
    assert_eq!(format!("{}", Token::Let), "let");
}

#[test]
fn token_display_double_colon() {
    assert_eq!(format!("{}", Token::DoubleColon), "::");
}

#[test]
fn token_display_bind_assign() {
    assert_eq!(format!("{}", Token::BindAssign), ":=");
}

#[test]
fn token_display_eof() {
    assert_eq!(format!("{}", Token::Eof), "<EOF>");
}
