#[cfg(test)]
mod tests {
    use crate::lexer::{lex, LexMode};
    use crate::parser::parse_module;
    use crate::parser::ast::*;

    #[test]
    fn test_compound_ident() {
        let src = "
action main
    let t tensor_result::Tensor
";
        let tokens = lex(src, LexMode::File).unwrap();
        let module = parse_module(tokens, src.len()).unwrap();
        // check that tensor_result::Tensor is a single Ident
    }

    #[test]
    fn test_list_cons() {
        let src = "
action main
    let l [head : tail]
";
        let tokens = lex(src, LexMode::File).unwrap();
        let module = parse_module(tokens, src.len()).unwrap();
    }
}
