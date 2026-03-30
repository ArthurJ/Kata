#[cfg(test)]
mod tests {
    use crate::parser::ast::{TypeRef, Spanned};
    use crate::type_checker::checker::TTopLevel;
    use crate::type_checker::tast::{TExpr, TStmt, TLiteral};
    use crate::codegen::compile_and_link;

    #[test]
    fn test_codegen_mvp_simple() {
        // TAST: action main () => Unit { let x = + 2 2 }
        
        let call_expr = TExpr::Call(
            Box::new((TExpr::Ident("+".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)),
            vec![
                (TExpr::Literal(TLiteral::Int(2)), 0..0),
                (TExpr::Literal(TLiteral::Int(2)), 0..0)
            ],
            TypeRef::Simple("Int".to_string())
        );

        let stmt_let = TStmt::Let(
            (crate::parser::ast::Pattern::Ident("x".to_string()), 0..0),
            (call_expr, 0..0)
        );

        let action_def = TTopLevel::ActionDef(
            "main".to_string(),
            vec![],
            (TypeRef::Simple("Unit".to_string()), 0..0),
            vec![(stmt_let, 0..0)],
            vec![]
        );

        let sig = TTopLevel::Signature(
            "main".to_string(),
            vec![],
            (TypeRef::Simple("Unit".to_string()), 0..0),
            vec![]
        );

        let add_sig = TTopLevel::Signature(
            "+".to_string(),
            vec![(TypeRef::Simple("Int".to_string()), 0..0), (TypeRef::Simple("Int".to_string()), 0..0)],
            (TypeRef::Simple("Int".to_string()), 0..0),
            vec![(crate::type_checker::directives::KataDirective::Ffi("kata_rt_add_int".to_string()), 0..0)]
        );

        let tast = vec![
            (add_sig, 0..0),
            (sig, 0..0),
            (action_def, 0..0),
        ];

        let out_bin = "target/test_codegen_simple";
        let result = compile_and_link(tast, out_bin);

        assert!(result.is_ok(), "Compilacao e Linkagem do MVP deve suceder. Erro: {:?}", result.err());

        // Cleanup
        let _ = std::fs::remove_file(format!("{}.o", out_bin));
    }
}
