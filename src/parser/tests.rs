#[cfg(test)]
mod tests {
    use crate::lexer::{lex, LexMode};
    use crate::parser::{parse_module, parse_expr, parse_stmt};
    use crate::parser::ast::*;

    fn parse(input: &str) -> Module {
        let tokens = lex(input, LexMode::File).expect("Lexing failed");
        parse_module(tokens, input.len()).expect("Parsing failed")
    }

    #[test]
    fn test_enum_predicates() {
        let input = "enum IMC\n    | Magreza(< _ 18.5)\n    | Normal(<= _ 25.0)\n    | Sobrepeso(<= _ 30.0)\n    | Obesidade";
        let module = parse(input);
        match &module.declarations[0].0 {
            TopLevel::Enum(name, variants, _) => {
                assert_eq!(name, "IMC");
                assert_eq!(variants.len(), 4);
                assert!(matches!(variants[0].data, VariantData::Predicate(_)));
                assert!(matches!(variants[3].data, VariantData::Unit));
            }
            _ => panic!("Expected Enum declaration"),
        }
    }

    #[test]
    fn test_enum_fixed_values() {
        let input = "enum StatusHTTP\n    | OK(200)\n    | NotFound(404)";
        let module = parse(input);
        match &module.declarations[0].0 {
            TopLevel::Enum(_, variants, _) => {
                assert_eq!(variants.len(), 2);
                match &variants[0].data {
                    VariantData::FixedValue(Expr::Int(v)) => assert_eq!(v, "200"),
                    _ => panic!("Expected FixedValue(200)"),
                }
            }
            _ => panic!("Expected Enum declaration"),
        }
    }

    #[test]
    fn test_pattern_matching_lambda() {
        let input = "fib :: Int => Int\nlambda 0: 0\nlambda 1: 1\nlambda n: + fib (- n 1) fib (- n 2)";
        let module = parse(input);
        assert_eq!(module.declarations.len(), 4);
        // Signature + 3 LambdaDefs
        match &module.declarations[1].0 {
            TopLevel::LambdaDef(args, _, _, _) => {
                match &args[0].0 {
                    Pattern::Literal(Expr::Int(v)) => assert_eq!(v, "0"),
                    _ => panic!("Expected Pattern::Literal(0)"),
                }
            }
            _ => panic!("Expected LambdaDef"),
        }
    }

    #[test]
    fn test_pattern_matching_match_action() {
        let input = "action test (r::Result::(Int, Err)) => Unit\n    match r\n        Ok v: echo! v\n        Err m: panic! m";
        let module = parse(input);
        match &module.declarations[0].0 {
            TopLevel::ActionDef(_, _, _, stmts, _) => {
                match &stmts[0].0 {
                    Stmt::Match(_, arms) => {
                        assert_eq!(arms.len(), 2);
                        match &arms[0] {
                            MatchArm::Pattern(pat, _) => {
                                match &pat.0 {
                                    Pattern::Sequence(pats) => {
                                        assert_eq!(pats.len(), 2);
                                        match &pats[0].0 {
                                            Pattern::Ident(id) => assert_eq!(id, "Ok"),
                                            _ => panic!("Expected Pattern::Ident(Ok)"),
                                        }
                                    }
                                    _ => panic!("Expected Pattern::Sequence for 'Ok v'"),
                                }
                            }
                        }
                    }
                    _ => panic!("Expected Stmt::Match"),
                }
            }
            _ => panic!("Expected ActionDef"),
        }
    }

    #[test]
    fn test_let_simple() {
        let input = "action test () => Unit\n    let x 10\n";
        let module = parse(input);
        assert_eq!(module.declarations.len(), 1);
    }

    #[test]
    fn test_let_destructuring() {
        let input = "action test () => Unit\n    let (a, b) (1, 2)\n";
        let module = parse(input);
        match &module.declarations[0].0 {
            TopLevel::ActionDef(_, _, _, stmts, _) => {
                match &stmts[0].0 {
                    Stmt::Let(pat, _) => {
                        assert!(matches!(pat.0, Pattern::Tuple(_)));
                    }
                    _ => panic!("Expected Stmt::Let with destructuring"),
                }
            }
            _ => panic!("Expected ActionDef"),
        }
    }

    #[test]
    fn test_repl_expression() {
        // No REPL, usuários podem omitir a quebra de linha final
        let input = "+ 2 2";
        let tokens = lex(input, LexMode::Repl).expect("Lexing failed in REPL mode");
        let (expr, _) = parse_expr(tokens, input.len()).expect("Failed to parse pure expression in REPL mode");
        
        match expr {
            Expr::Sequence(atoms) => {
                assert_eq!(atoms.len(), 3);
                match &atoms[0].0 {
                    Expr::Ident(s) => assert_eq!(s, "+"),
                    _ => panic!("Expected ident +"),
                }
            }
            _ => panic!("Expected Expr::Sequence"),
        }
    }

    #[test]
    fn test_repl_statement() {
        let input = "let x 5";
        let tokens = lex(input, LexMode::Repl).expect("Lexing failed in REPL mode");
        let (stmt, _) = parse_stmt(tokens, input.len()).expect("Failed to parse pure statement in REPL mode");
        
        match stmt {
            Stmt::Let(pat, _) => {
                match pat.0 {
                    Pattern::Ident(id) => assert_eq!(id, "x"),
                    _ => panic!("Expected pattern ident x")
                }
            }
            _ => panic!("Expected Stmt::Let"),
        }
    }

    #[test]
    fn test_line_continuation() {
        let input = "action test () => Unit\n    let x \\ \n 10\n";
        let module = parse(input);
        match &module.declarations[0].0 {
            TopLevel::ActionDef(_, _, _, stmts, _) => {
                assert_eq!(stmts.len(), 1);
                match &stmts[0].0 {
                    Stmt::Let(_, expr) => {
                        match &expr.0 {
                            Expr::Int(v) => assert_eq!(v, "10"),
                            _ => panic!("Expected value 10 in continuation"),
                        }
                    },
                    _ => panic!("Expected Stmt::Let"),
                }
            }
            _ => panic!("Expected ActionDef"),
        }
    }

    #[test]
    fn test_newlines_inside_delimiters() {
        let input = "action test () => Unit\n    let lista [\n        1\n        2\n    ]\n";
        let module = parse(input);
        match &module.declarations[0].0 {
            TopLevel::ActionDef(_, _, _, stmts, _) => {
                match &stmts[0].0 {
                    Stmt::Let(_, expr) => {
                        match &expr.0 {
                            Expr::List(items) => assert_eq!(items.len(), 2),
                            _ => panic!("Expected List with 2 items"),
                        }
                    },
                    _ => panic!("Expected Stmt::Let"),
                }
            }
            _ => panic!("Expected ActionDef"),
        }
    }

    #[test]
    fn test_comments() {
        let input = "action test () => Unit\n    # Isso é um comentário\n    let x 10\n";
        let module = parse(input);
        assert_eq!(module.declarations.len(), 1);
        match &module.declarations[0].0 {
            TopLevel::ActionDef(_, _, _, stmts, _) => {
                assert_eq!(stmts.len(), 1); // Only `let x 10` remains
            }
            _ => panic!("Expected ActionDef"),
        }
    }

    #[test]
    fn test_postfix_try() {
        let input = "action test () => Unit\n    let val ler_arquivo! caminho ?\n";
        let module = parse(input);
        match &module.declarations[0].0 {
            TopLevel::ActionDef(_, _, _, stmts, _) => {
                match &stmts[0].0 {
                    Stmt::Let(_, expr) => {
                        match &expr.0 {
                            Expr::Sequence(atoms) => {
                                assert_eq!(atoms.len(), 2);
                                match &atoms[1].0 {
                                    Expr::Try(inner) => {
                                        match &inner.0 {
                                            Expr::Ident(id) => assert_eq!(id, "caminho"),
                                            _ => panic!("Expected inner Ident"),
                                        }
                                    }
                                    _ => panic!("Expected Try expression"),
                                }
                            }
                            _ => panic!("Expected Sequence"),
                        }
                    },
                    _ => panic!("Expected Stmt::Let"),
                }
            }
            _ => panic!("Expected ActionDef"),
        }
    }
}
