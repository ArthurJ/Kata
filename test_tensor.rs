#[cfg(test)]
mod tests {
    use crate::lexer::{lex, LexMode};
    use crate::parser::parse_module;
    use crate::parser::ast::*;

    #[test]
    fn test_tensor_type() {
        let src = "
action main
    let t Tensor::(NUM, (Int...))
";
        let tokens = lex(src, LexMode::File).unwrap();
        let module = parse_module(tokens, src.len()).unwrap();
        println!("{:#?}", module);
    }
}
