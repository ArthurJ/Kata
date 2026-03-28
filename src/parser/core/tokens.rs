use crate::lexer::Token;
use crate::parser::error::ParserError;
use chumsky::prelude::*;

pub fn ident_string() -> impl Parser<Token, String, Error = ParserError> + Clone {
    filter_map(|span, tok| match tok {
        Token::Ident(s) => Ok(s),
        Token::TypeID(s) => Ok(s),
        Token::TypeVar(s) => Ok(s),
        Token::InterfaceID(s) => Ok(s),
        _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
    })
}
