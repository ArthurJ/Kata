//! Shared helpers for parser integration tests.

use kata_ast::Item;
use kata_lexer::lex;
use kata_parser::parse;

pub(super) fn parse_src(src: &str) -> kata_ast::Module {
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap()
}

pub(super) fn first_item(m: &kata_ast::Module) -> &Item {
    &m.items.first().expect("at least one item").node
}