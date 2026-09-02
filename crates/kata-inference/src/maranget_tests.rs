use super::*;
use kata_core::EnumRegistry;
use kata_core::ty::PrimTy;

fn env_bool() -> EnumRegistry {
    let mut reg = EnumRegistry::new();
    reg.register(
        "core",
        "Boolean",
        vec![
            kata_core::VariantInfo {
                name: "True".to_string(),
                payload_ty: None,
                predicate: None,
                fixed_value: None,
            },
            kata_core::VariantInfo {
                name: "False".to_string(),
                payload_ty: None,
                predicate: None,
                fixed_value: None,
            },
        ],
    );
    reg
}

fn env_optional() -> EnumRegistry {
    let mut reg = env_bool();
    reg.register(
        "core",
        "Optional",
        vec![
            kata_core::VariantInfo {
                name: "Some".to_string(),
                payload_ty: Some(Ty::Prim(PrimTy::Int)),
                predicate: None,
                fixed_value: None,
            },
            kata_core::VariantInfo {
                name: "None".to_string(),
                payload_ty: None,
                predicate: None,
                fixed_value: None,
            },
        ],
    );
    reg
}

fn variant(enum_name: &str, name: &str) -> TypedPattern {
    TypedPattern::Variant {
        enum_name: enum_name.to_string(),
        variant: name.to_string(),
        sub_patterns: None,
        tag: 0,
    }
}

fn variant_with(enum_name: &str, name: &str, sub: TypedPattern) -> TypedPattern {
    TypedPattern::Variant {
        enum_name: enum_name.to_string(),
        variant: name.to_string(),
        sub_patterns: Some(vec![kata_ast::Spanned::new(sub, kata_ast::Span::zero())]),
        tag: 0,
    }
}

fn wildcard() -> TypedPattern {
    TypedPattern::Wildcard
}

fn int_literal(text: &str) -> TypedPattern {
    TypedPattern::Literal {
        value: kata_ast::Spanned::new(
            crate::typed::TypedExpr {
                span: kata_ast::Span::zero(),
                ty: Ty::Prim(PrimTy::Int),
                tail_pos: false,
                escape: kata_core::EscapeTarget::Local,
                kind: crate::typed::TypedExprKind::IntLit {
                    text: text.to_string(),
                },
            },
            kata_ast::Span::zero(),
        ),
    }
}

#[test]
fn test_bool_exhaustive() {
    let reg = env_bool();
    let patterns = vec![
        vec![variant("Boolean", "True")],
        vec![variant("Boolean", "False")],
    ];
    let result = check_exhaustiveness_maranget(
        &patterns,
        &[Ty::Sum("Boolean".to_string())],
        false,
        &reg,
        None,
        None,
        None,
    );
    assert!(result.exhaustive, "True + False should be exhaustive");
}

#[test]
fn test_bool_not_exhaustive() {
    let reg = env_bool();
    let patterns = vec![vec![variant("Boolean", "True")]];
    let result = check_exhaustiveness_maranget(
        &patterns,
        &[Ty::Sum("Boolean".to_string())],
        false,
        &reg,
        None,
        None,
        None,
    );
    assert!(!result.exhaustive, "Only True should not be exhaustive");
    assert_eq!(result.missing, vec!["False"]);
}

#[test]
fn test_bool_with_wildcard_exhaustive() {
    let reg = env_bool();
    let patterns = vec![vec![variant("Boolean", "True")], vec![wildcard()]];
    let result = check_exhaustiveness_maranget(
        &patterns,
        &[Ty::Sum("Boolean".to_string())],
        false,
        &reg,
        None,
        None,
        None,
    );
    assert!(result.exhaustive, "True + _ should be exhaustive");
}

#[test]
fn test_optional_exhaustive() {
    let reg = env_optional();
    let patterns = vec![
        vec![variant_with("Optional", "Some", wildcard())],
        vec![variant("Optional", "None")],
    ];
    let result = check_exhaustiveness_maranget(
        &patterns,
        &[Ty::Generic("Optional".to_string(), vec![])],
        false,
        &reg,
        None,
        None,
        None,
    );
    assert!(
        result.exhaustive,
        "Some _ + None should be exhaustive for Optional"
    );
}

#[test]
fn test_optional_not_exhaustive_missing_some() {
    let reg = env_optional();
    // Only None — missing Some
    let patterns = vec![vec![variant("Optional", "None")]];
    let result = check_exhaustiveness_maranget(
        &patterns,
        &[Ty::Generic("Optional".to_string(), vec![])],
        false,
        &reg,
        None,
        None,
        None,
    );
    assert!(!result.exhaustive, "Only None should not be exhaustive");
    assert_eq!(result.missing, vec!["Some (_)"]);
}

#[test]
fn test_redundant_arm() {
    let reg = env_bool();
    let patterns = vec![
        vec![variant("Boolean", "True")],
        vec![variant("Boolean", "False")],
        vec![variant("Boolean", "True")], // redundant
    ];
    let col = [Ty::Sum("Boolean".to_string())];
    assert!(!is_arm_redundant(&patterns, &col, 0, &reg));
    assert!(!is_arm_redundant(&patterns, &col, 1, &reg));
    assert!(is_arm_redundant(&patterns, &col, 2, &reg));
}

#[test]
fn test_not_redundant_arm() {
    let reg = env_bool();
    let patterns = vec![
        vec![variant("Boolean", "True")],
        vec![variant("Boolean", "False")],
    ];
    let col = [Ty::Sum("Boolean".to_string())];
    assert!(!is_arm_redundant(&patterns, &col, 1, &reg));
}

#[test]
fn test_int_requires_wildcard() {
    let reg = env_bool();
    let patterns = vec![vec![int_literal("0")]];
    let result = check_exhaustiveness_maranget(
        &patterns,
        &[Ty::Prim(PrimTy::Int)],
        false,
        &reg,
        None,
        None,
        None,
    );
    assert!(
        !result.exhaustive,
        "Single literal on Int should not be exhaustive"
    );
}

#[test]
fn test_int_with_wildcard_exhaustive() {
    let reg = env_bool();
    let patterns = vec![vec![int_literal("0")], vec![wildcard()]];
    let result = check_exhaustiveness_maranget(
        &patterns,
        &[Ty::Prim(PrimTy::Int)],
        true,
        &reg,
        None,
        None,
        None,
    );
    assert!(result.exhaustive, "literal + wildcard should be exhaustive");
}
