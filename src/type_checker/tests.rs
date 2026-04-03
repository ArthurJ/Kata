
use crate::parser::ast::{Module, TopLevel, Spanned, TypeRef, Pattern};
use crate::type_checker::Checker;

#[cfg(test)]
mod tests {
    use super::*;

    fn check(src: &str) -> Checker {
        let tokens = crate::lexer::lex(src, crate::lexer::LexMode::File).unwrap();
        let module = crate::parser::parse_module(tokens, src.len()).unwrap();
        let mut checker = Checker::new();
        checker.check_module(&module);
        checker
    }

    #[test]
    fn test_multiple_dispatch_success() {
        let src = "
soma :: Int Int => Int
lambda a b: + a b

action main
    let x soma 10 20
";
        let mut checker = Checker::new();
        checker.env.define("+".to_string(), 2, TypeRef::Function(
            vec![
                (TypeRef::Simple("Int".to_string()), 0..0),
                (TypeRef::Simple("Int".to_string()), 0..0)
            ],
            Box::new((TypeRef::Simple("Int".to_string()), 0..0))
        ), false, true, None);

        let tokens = crate::lexer::lex(src, crate::lexer::LexMode::File).unwrap();
        let module = crate::parser::parse_module(tokens, src.len()).unwrap();
        checker.check_module(&module);

        assert!(checker.errors.is_empty(), "Erros encontrados: {:?}", checker.errors);
    }

    #[test]
    fn test_multiple_dispatch_fail() {
        let src = "
soma :: Int Int => Int
lambda a b: + a b

action main
    let x soma 10 \"texto\"
";
        let mut checker = Checker::new();
        checker.env.define("+".to_string(), 2, TypeRef::Function(
            vec![
                (TypeRef::Simple("Int".to_string()), 0..0),
                (TypeRef::Simple("Int".to_string()), 0..0)
            ],
            Box::new((TypeRef::Simple("Int".to_string()), 0..0))
        ), false, true, None);

        let tokens = crate::lexer::lex(src, crate::lexer::LexMode::File).unwrap();
        let module = crate::parser::parse_module(tokens, src.len()).unwrap();
        checker.check_module(&module);

        assert!(checker.errors.iter().any(|e| e.0.to_string().contains("TypeError")), "Deveria ter erro de tipo.");
    }

    #[test]
    fn test_purity_barrier() {
        let src = "
action log! (msg) => ()

soma :: Int Int => Int
lambda a b:
    log!(\"tentando logar no lambda\")
    + a b
";
        let checker = check(src);
        assert!(checker.errors.iter().any(|e| e.0.to_string().contains("PurityError")), "Deveria ter erro de pureza.");
    }

    #[test]
    fn test_action_recursion() {
        let src = "
action recursiva ()
    recursiva!()
";
        let checker = check(src);
        assert!(checker.errors.iter().any(|e| e.0.to_string().contains("Recursao proibida")), "Deveria ter erro de recursao.");
    }

    #[test]
    fn test_match_exhaustiveness() {
        let src = "
enum Status
    | Ok
    | Err

action processar (s::Status)
    match s
        Ok: ()
";
        let checker = check(src);
        assert!(checker.errors.iter().any(|e| e.0.to_string().contains("nao eh exaustivo")), "Deveria ter erro de exaustividade.");
    }

    #[test]
    fn test_orphan_rule() {
        let src = "
ExtType implements ExtInterface
    metodo :: ExtType => ()
    lambda _: ()
";
        let checker = check(src);
        assert!(checker.errors.iter().any(|e| e.0.to_string().contains("OrphanRuleError")), "Deveria ter violado a Orphan Rule.");
    }

    #[test]
    fn test_specificity_score() {
        let src = "
interface NUM
    + :: NUM NUM => NUM

soma :: Int Int => Int
lambda a b: 1

soma :: NUM NUM => NUM
lambda a b: a

action main
    let x soma 10 20
";
        let mut checker = Checker::new();
        let tokens = crate::lexer::lex(src, crate::lexer::LexMode::File).unwrap();
        let module = crate::parser::parse_module(tokens, src.len()).unwrap();
        checker.check_module(&module);
        assert!(checker.errors.is_empty());
    }

    #[test]
    fn test_multiple_dispatch_ambiguity() {
        let src = "
processar :: T A => Int
lambda a b: 1

processar :: A T => Text
lambda a b: \"ambiguo\"

action main
    let a processar 10 20
";
        let checker = check(src);
        assert!(checker.errors.iter().any(|e| e.0.to_string().contains("AmbiguityError")), "Deveria ter detectado ambiguidade.");
    }

    #[test]
    fn test_commutative_dispatch() {
        let src = "
@commutative
+ :: Int Float => Float
lambda a b: b

action main
    let x + 5.0 10
";
        let checker = check(src);
        assert!(checker.errors.is_empty(), "Erros encontrados no teste comutativo: {:?}", checker.errors);
    }

    #[test]
    fn test_early_checking_generics() {
        let src = "
interface NUM
    + :: NUM NUM => NUM

soma_gen :: T T => T
lambda a b: + a b
with
    T as NUM
";
        let checker = check(src);
        assert!(checker.errors.is_empty(), "Erros encontrados no Early Checking: {:?}", checker.errors);

        let src_fail = "
interface SHOW
    str :: SHOW => Text

interface NUM implements SHOW
    + :: NUM NUM => NUM

minha_funcao :: T => Text
lambda x: + x x
with
    T as SHOW
";
        let checker_fail = check(src_fail);
        assert!(!checker_fail.errors.is_empty(), "O TypeChecker permitiu acesso ao '+' para um tipo que so assina SHOW");
        assert!(checker_fail.errors.iter().any(|e| e.0.to_string().contains("TypeError") || e.0.to_string().contains("Type Mismatch")), "Erros reais: {:?}", checker_fail.errors);
    }

    #[test]
    fn test_predicate_inheritance() {
        use crate::type_checker::tast::TExpr;
        use crate::parser::ast::TypeRef;

        let source = "
data (Int, > _ 0) as PositiveInt
data (Int, != _ 0) as NonZeroInt

test_inherit :: PositiveInt NonZeroInt => Result::((Int, > _ 0, != _ 0), Text)
lambda a b: + a b
";
        let tokens = crate::lexer::lex(source, crate::lexer::LexMode::File).unwrap();
        let module = crate::parser::parse_module(tokens, source.len()).unwrap();
        let mut checker = crate::type_checker::Checker::new();
        
        checker.env.define("+".to_string(), 2, TypeRef::Function(
            vec![
                (TypeRef::Simple("Int".to_string()), 0..0),
                (TypeRef::Simple("Int".to_string()), 0..0)
            ],
            Box::new((TypeRef::Simple("Int".to_string()), 0..0))
        ), false, true, None);

        checker.check_module(&module);
        
        assert!(checker.errors.is_empty(), "Erros Semanticos: {:?}", checker.errors);
        
        if let crate::type_checker::checker::TTopLevel::LambdaDef(_, t_body, _, _) = &checker.tast[3].0 {
            if let TExpr::Sequence(seq, _) = &t_body.0 {
                if let Some((TExpr::Call(_, _, call_ty), _)) = seq.last() {
                    if let TypeRef::Generic(res, args) = call_ty {
                        assert_eq!(res, "Result");
                        if let TypeRef::Refined(base, preds) = &args[0].0 {
                            assert_eq!(base.0, TypeRef::Simple("Int".to_string()));
                            assert_eq!(preds.len(), 2);
                        } else {
                            panic!("Esperado TypeRef::Refined no primeiro argumento do Result, encontrado: {:?}", args[0].0);
                        }
                    } else {
                        panic!("Esperado TypeRef::Generic('Result'), encontrado: {:?}", call_ty);
                    }
                }
            }
        }
    }

    #[test]
    fn test_lambda_exhaustiveness() {
        let src = "
enum Status
    | Ok
    | Err

processar :: Status => Int
lambda Ok: 1
";
        let tokens = crate::lexer::lex(src, crate::lexer::LexMode::File).unwrap();
        let module = crate::parser::parse_module(tokens, src.len()).unwrap();
        let mut checker = crate::type_checker::Checker::new();
        checker.check_module(&module);
        assert!(checker.errors.iter().any(|e| e.0.to_string().contains("nao eh exaustivo")), "Deveria ter erro de exaustividade no lambda.");
    }

    #[test]
    fn test_tast_channels_and_try() {
        use crate::type_checker::tast::TExpr;
        let src = "
action test_csp (tx rx)
    !> tx 10
    let a <! rx
    let val a?
    ()
";
        let tokens = crate::lexer::lex(src, crate::lexer::LexMode::File).unwrap();
        let module = crate::parser::parse_module(tokens, src.len()).unwrap();
        let mut checker = crate::type_checker::Checker::new();
        checker.check_module(&module);
        
        assert!(checker.errors.is_empty(), "Erros: {:?}", checker.errors);
        
        if let crate::type_checker::checker::TTopLevel::ActionDef(_, _, _, body, _) = &checker.tast[0].0 {
            if let crate::type_checker::tast::TStmt::Expr((TExpr::Sequence(seq, _), _)) = &body[0].0 {
                if let TExpr::ChannelSend(..) = &seq[0].0 {
                    // Ok
                } else { panic!("Esperado ChannelSend dentro de Sequence, encontrado: {:?}", seq[0].0); }
            } else { panic!("Esperado Expr com Sequence, encontrado: {:?}", body[0].0); }
            
            if let crate::type_checker::tast::TStmt::Let(_, (TExpr::Sequence(seq, _), _)) = &body[1].0 {
                if let TExpr::ChannelRecv(..) = &seq[0].0 {
                    // Ok
                } else { panic!("Esperado ChannelRecv dentro de Sequence, encontrado: {:?}", seq[0].0); }
            } else { panic!("Esperado Let com Sequence, encontrado: {:?}", body[1].0); }
            
            if let crate::type_checker::tast::TStmt::Let(_, (TExpr::Try(..), _)) = &body[2].0 {
                // Ok
            } else { panic!("Esperado Try dentro de Let, encontrado: {:?}", body[2].0); }
        }
    }

    #[test]
    fn test_auto_expand_exports_and_imports() {
        let mod_a_src = "
interface TEST_IFACE
    metodo_iface :: TEST_IFACE => Int

data MeuTipo (x)

MeuTipo implements TEST_IFACE
    metodo_iface :: MeuTipo => Int
    lambda a: 42
    
    metodo_extra :: MeuTipo => Text
    lambda a: \"extra\"

export MeuTipo
";
        
        let mod_b_src = "
import mod_a.(MeuTipo)

action main
    let t MeuTipo 10
    let x metodo_extra t
    let y metodo_iface t
";

        let tokens_a = crate::lexer::lex(mod_a_src, crate::lexer::LexMode::File).unwrap();
        let ast_a = crate::parser::parse_module(tokens_a, mod_a_src.len()).unwrap();
        
        let mut checker_a = Checker::new();
        checker_a.check_module(&ast_a);
        assert!(checker_a.errors.is_empty(), "Mod A: {:?}", checker_a.errors);
        
        let tokens_b = crate::lexer::lex(mod_b_src, crate::lexer::LexMode::File).unwrap();
        let ast_b = crate::parser::parse_module(tokens_b, mod_b_src.len()).unwrap();
        
        let mut checker_b = Checker::new();
        checker_b.compiled_modules.insert("mod_a".to_string(), checker_a.env.clone());
        
        checker_b.check_module(&ast_b); 
        
        let mut final_checker = Checker::new();
        final_checker.compiled_modules = checker_b.compiled_modules.clone();
        
        for (path, specific) in &checker_b.env.imports {
            if let Some(target_module_name) = path.split('.').next() {
                if let Some(target_env) = final_checker.compiled_modules.get(target_module_name) {
                    final_checker.env.import_from(target_env, target_module_name, specific);
                }
            }
        }
        
        final_checker.check_module(&ast_b);
        assert!(final_checker.errors.is_empty(), "Erros encontrados ao importar o tipo pai: {:?}", final_checker.errors);
    }
}
