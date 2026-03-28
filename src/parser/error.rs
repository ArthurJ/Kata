use crate::lexer::Token;
use chumsky::error::Simple;

pub type ParserError = Simple<Token>;
