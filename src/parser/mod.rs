pub mod ast;
pub mod core;
pub mod error;
pub mod grammar;

#[cfg(test)]
mod tests;

use chumsky::Parser;
use chumsky::Stream;

pub fn parse_module(
    tokens: Vec<(crate::lexer::Token, std::ops::Range<usize>)>,
    source_len: usize,
) -> Result<ast::Module, Vec<error::ParserError>> {
    let stream = Stream::from_iter(source_len..source_len, tokens.into_iter());
    grammar::module::module_parser().parse(stream)
}

#[allow(dead_code)]
pub fn parse_expr(
    tokens: Vec<(crate::lexer::Token, std::ops::Range<usize>)>,
    source_len: usize,
) -> Result<ast::Spanned<ast::Expr>, Vec<error::ParserError>> {
    let stream = Stream::from_iter(source_len..source_len, tokens.into_iter());
    grammar::expr::expr_parser()
        .then_ignore(chumsky::prelude::just(crate::lexer::Token::Newline).repeated())
        .then_ignore(chumsky::prelude::end())
        .parse(stream)
}

#[allow(dead_code)]
pub fn parse_stmt(
    tokens: Vec<(crate::lexer::Token, std::ops::Range<usize>)>,
    source_len: usize,
) -> Result<ast::Spanned<ast::Stmt>, Vec<error::ParserError>> {
    let stream = Stream::from_iter(source_len..source_len, tokens.into_iter());
    grammar::stmt::stmt_parser()
        .then_ignore(chumsky::prelude::just(crate::lexer::Token::Newline).repeated())
        .then_ignore(chumsky::prelude::end())
        .parse(stream)
}
mod tests_import;
