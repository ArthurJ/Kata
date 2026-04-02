#[cfg(test)]
mod tests {
    use crate::parser::ast::{TypeRef, Spanned, Pattern};
    use crate::type_checker::checker::TTopLevel;
    use crate::type_checker::tast::{TExpr, TStmt, TLiteral, TMatchArm, AllocMode};
    use crate::codegen::compile_and_link;
    use std::fs;

    fn dummy_span() -> crate::parser::ast::Span { 0..0 }

    #[test]
    fn test_codegen_loop_break() {
        // action main () => Unit { 
        //   var i 0
        //   loop {
        //     match (== i 5) {
        //       True: break
        //       otherwise: i = + i 1
        //     }
        //   }
        // }
        
        let var_i = TStmt::Var("i".to_string(), (TExpr::Literal(TLiteral::Int(0)), dummy_span()));
        
        let cond_expr = TExpr::Call(
            Box::new((TExpr::Ident("=".to_string(), TypeRef::Function(vec![(TypeRef::Simple("Int".to_string(, _)), 0..0), (TypeRef::Simple("Int".to_string()), 0..0)], Box::new((TypeRef::Simple("Bool".to_string()), 0..0)))), dummy_span())),
            vec![
                (TExpr::Ident("i".to_string(), TypeRef::Simple("Int".to_string()), crate::type_checker::tast::AllocMode::Local), dummy_span()),
                (TExpr::Literal(TLiteral::Int(5)), dummy_span())
            ],
            TypeRef::Simple("Bool".to_string())
        );

        let match_stmt = TStmt::Match(
            (cond_expr, dummy_span()),
            vec![
                TMatchArm {
                    pattern: (Pattern::Ident("True".to_string()), dummy_span()),
                    body: vec![(TStmt::Break, dummy_span())],
                },
                TMatchArm {
                    pattern: (Pattern::Ident("otherwise".to_string()), dummy_span()),
                    body: vec![
                        (TStmt::Var("i".to_string(), (TExpr::Call(
                            Box::new((TExpr::Ident("+".to_string(), TypeRef::Function(vec![(TypeRef::Simple("Int".to_string(, _)), 0..0), (TypeRef::Simple("Int".to_string()), 0..0)], Box::new((TypeRef::Simple("Int".to_string()), 0..0)))), dummy_span())),
                            vec![
                                (TExpr::Ident("i".to_string(), TypeRef::Simple("Int".to_string()), crate::type_checker::tast::AllocMode::Local), dummy_span()),
                                (TExpr::Literal(TLiteral::Int(1)), dummy_span())
                            ],
                            TypeRef::Simple("Int".to_string())
                        ), dummy_span())), dummy_span())
                    ],
                }
            ]
        );

        let loop_stmt = TStmt::Loop(vec![(match_stmt, dummy_span())]);

        let action_def = TTopLevel::ActionDef(
            "main".to_string(),
            vec![],
            (TypeRef::Simple("Unit".to_string()), dummy_span()),
            vec![(var_i, dummy_span()), (loop_stmt, dummy_span())],
            vec![]
        );

        let tast = vec![
            (TTopLevel::Signature("=".to_string(), vec![(TypeRef::Simple("Int".to_string()), 0..0), (TypeRef::Simple("Int".to_string()), 0..0)], (TypeRef::Simple("Bool".to_string()), 0..0), vec![(crate::type_checker::directives::KataDirective::Ffi("kata_rt_eq_int".to_string()), 0..0)]), dummy_span()),
            (TTopLevel::Signature("+".to_string(), vec![(TypeRef::Simple("Int".to_string()), 0..0), (TypeRef::Simple("Int".to_string()), 0..0)], (TypeRef::Simple("Int".to_string()), 0..0), vec![(crate::type_checker::directives::KataDirective::Ffi("kata_rt_add_int".to_string()), 0..0)]), dummy_span()),
            (action_def, dummy_span()),
        ];

        let out_bin = "target/test_codegen_loop";
        let result = compile_and_link(tast, &crate::type_checker::environment::TypeEnv::new(), out_bin);

        assert!(result.is_ok(), "Erro: {:?}", result.err());
        let _ = fs::remove_file(format!("{}.o", out_bin));
    }

    #[test]
    fn test_codegen_for_list() {
        // action main () => Unit {
        //   let list [1, 2, 3]
        //   for x in list {
        //     echo!(str x)
        //   }
        // }
        
        let list_expr = TExpr::List(
            vec![
                (TExpr::Literal(TLiteral::Int(1)), dummy_span()),
                (TExpr::Literal(TLiteral::Int(2)), dummy_span()),
                (TExpr::Literal(TLiteral::Int(3)), dummy_span()),
            ],
            TypeRef::Generic("List".to_string(), vec![(TypeRef::Simple("Int".to_string()), 0..0)]),
            AllocMode::Local
        );

        let let_list = TStmt::Let(
            (Pattern::Ident("list".to_string()), dummy_span()),
            (list_expr, dummy_span())
        );

        let str_call = TExpr::Call(
            Box::new((TExpr::Ident("str".to_string(), TypeRef::Function(vec![(TypeRef::Simple("Int".to_string(, _)), 0..0)], Box::new((TypeRef::Simple("Text".to_string()), 0..0)))), dummy_span())),
            vec![(TExpr::Ident("x".to_string(), TypeRef::Simple("Int".to_string()), crate::type_checker::tast::AllocMode::Local), dummy_span())],
            TypeRef::Simple("Text".to_string())
        );

        let echo_call = TStmt::Expr((TExpr::Call(
            Box::new((TExpr::Ident("echo!".to_string(), TypeRef::Function(vec![(TypeRef::Simple("Text".to_string(, _)), 0..0)], Box::new((TypeRef::Simple("Unit".to_string()), 0..0)))), dummy_span())),
            vec![(str_call, dummy_span())],
            TypeRef::Simple("Unit".to_string())
        ), dummy_span()));

        let for_stmt = TStmt::For(
            "x".to_string(),
            (TExpr::Ident("list".to_string(), TypeRef::Generic("List".to_string(, crate::type_checker::tast::AllocMode::Local), vec![(TypeRef::Simple("Int".to_string()), 0..0)])), dummy_span()),
            vec![(echo_call, dummy_span())]
        );

        let action_def = TTopLevel::ActionDef(
            "main".to_string(),
            vec![],
            (TypeRef::Simple("Unit".to_string()), dummy_span()),
            vec![(let_list, dummy_span()), (for_stmt, dummy_span())],
            vec![]
        );

        let tast = vec![
            (TTopLevel::Signature("str".to_string(), vec![(TypeRef::Simple("Int".to_string()), 0..0)], (TypeRef::Simple("Text".to_string()), 0..0), vec![(crate::type_checker::directives::KataDirective::Ffi("kata_rt_int_to_str".to_string()), 0..0)]), dummy_span()),
            (TTopLevel::Signature("echo!".to_string(), vec![(TypeRef::Simple("Text".to_string()), 0..0)], (TypeRef::Simple("Unit".to_string()), 0..0), vec![(crate::type_checker::directives::KataDirective::Ffi("kata_rt_print_str".to_string()), 0..0)]), dummy_span()),
            (action_def, dummy_span()),
        ];

        let out_bin = "target/test_codegen_for";
        let result = compile_and_link(tast, &crate::type_checker::environment::TypeEnv::new(), out_bin);

        assert!(result.is_ok(), "Erro: {:?}", result.err());
        let _ = fs::remove_file(format!("{}.o", out_bin));
    }
}
