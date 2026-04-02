use kata::lexer::{lex, LexMode};

fn main() {
    let src = "data Person (age::Int)";
    let tokens = lex(src, LexMode::File).unwrap();
    println!("{:?}", tokens);
}
