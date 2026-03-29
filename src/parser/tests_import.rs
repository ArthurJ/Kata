use super::ast::*;
use chumsky::Parser;
use crate::lexer::lex;
use crate::lexer::LexMode;
use super::grammar::module::module_parser;


fn parse(input: &str) -> Module {
    let tokens = lex(input, LexMode::File).unwrap();
    let len = input.len();
    let stream = chumsky::Stream::from_iter(len..len, tokens.into_iter());
    module_parser().parse(stream).unwrap()
}

#[test]
fn test_import_as() {
    let input = "import io.(echo as print)\nimport utils.math.(soma as add, sub)";
    let m = parse(input);
    
    assert_eq!(m.declarations.len(), 2);
    
    match &m.declarations[0].0 {
        TopLevel::Import(path, specific) => {
            assert_eq!(path, "io");
            assert_eq!(specific.len(), 1);
            assert_eq!(specific[0].0, "echo");
            assert_eq!(specific[0].1, Some("print".to_string()));
        }
        _ => panic!("Expected Import"),
    }
    
    match &m.declarations[1].0 {
        TopLevel::Import(path, specific) => {
            assert_eq!(path, "utils.math");
            assert_eq!(specific.len(), 2);
            assert_eq!(specific[0].0, "soma");
            assert_eq!(specific[0].1, Some("add".to_string()));
            assert_eq!(specific[1].0, "sub");
            assert_eq!(specific[1].1, None);
        }
        _ => panic!("Expected Import"),
    }
}
