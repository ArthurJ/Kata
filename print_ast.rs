use kata::lexer::{lex, LexMode};
use kata::parser::parse_module;

fn main() {
    let src = "data Person (age::Int)";
    let tokens = lex(src, LexMode::File).unwrap();
    println!("{:?}", tokens);
    let ast = parse_module(tokens, src.len());
    println!("{:?}", ast);
}
