
#[cfg(test)]
mod tests {
    use super::*;

    fn check(src: &str) -> Checker {
        let tokens = crate::lexer::lex(src, crate::lexer::LexMode::File).unwrap();
        let module = crate::parser::parse_module(tokens, src.len()).unwrap();
        let mut checker = Checker::new();
        checker.check_module(&module);
        println!("AST PARSED: {:#?}", module);
        println!("ERROS: {:?}", checker.errors);
        checker
    }

    #[test]
    fn test_refined_scoring_generics_vs_interfaces() {
        let src = "
interface SHOW
    str :: SHOW => Text

interface NUM implements SHOW
    + :: NUM NUM => NUM

data Person (age::Int)

Person implements SHOW
    str :: Person => Text
    lambda p: \"Person\"

# Funcao com Interface (Deve ter score 5)
imprimir :: T => Text
lambda x: str x
with
    T as SHOW

# Funcao Concreta (Deve ter score 10 e vencer)
imprimir :: Person => Text
lambda p: \"p\"

# Funcao Generica Pura (Deve ter score 1)
imprimir :: A => Text
lambda x: \"qualquer\"

action main
    # Agora isso deve funcionar com o construtor oficial do DataDef!
    let p Person 30
    
    # Isso deve chamar 'imprimir :: Person => Text' (score 10) 
    # e nao emitir o erro 'Erro Semantico (Ambiguidade) ao chamar `imprimir`'
    let c imprimir p
";
        let checker = check(src);
        assert!(checker.errors.is_empty(), "Erros encontrados no teste de scoring: {:?}", checker.errors);
    }
}
