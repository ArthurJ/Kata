use super::*;

fn v(name: &str) -> VariantInfo {
    VariantInfo {
        name: name.into(),
        payload_ty: None,
        predicate: None,
        fixed_value: None,
    }
}

fn v_with_payload(name: &str, ty: Ty) -> VariantInfo {
    VariantInfo {
        name: name.into(),
        payload_ty: Some(ty),
        predicate: None,
        fixed_value: None,
    }
}

#[test]
fn register_and_query() {
    let mut registry = EnumRegistry::new();
    registry.register("core", "Boolean", vec![v("True"), v("False")]);

    assert!(registry.is_variant("Boolean", "True"));
    assert!(registry.is_variant("Boolean", "False"));
    assert!(!registry.is_variant("Boolean", "Maybe"));

    let variants = registry.variants_of("Boolean");
    assert_eq!(variants, &["True", "False"]);
}

#[test]
fn find_enum_of_variant() {
    let mut registry = EnumRegistry::new();
    registry.register("core", "Boolean", vec![v("True"), v("False")]);

    assert_eq!(registry.find_enum_of_variant("True"), Some("Boolean"));
    assert_eq!(registry.find_enum_of_variant("False"), Some("Boolean"));
    assert_eq!(registry.find_enum_of_variant("Maybe"), None);
}

#[test]
fn find_enums_with_variant() {
    let mut registry = EnumRegistry::new();
    registry.register("core", "Boolean", vec![v("True"), v("False")]);
    registry.register("user", "Flag", vec![v("True"), v("Off")]);

    let mut enums = registry.find_enums_with_variant("True");
    enums.sort();
    assert_eq!(enums, vec!["Boolean", "Flag"]);

    assert_eq!(registry.find_enums_with_variant("False"), vec!["Boolean"]);
    assert_eq!(registry.find_enums_with_variant("Off"), vec!["Flag"]);
    assert!(registry.find_enums_with_variant("Maybe").is_empty());
}

#[test]
fn unknown_enum_returns_empty() {
    let registry = EnumRegistry::new();
    assert!(registry.variants_of("NonExistent").is_empty());
    assert!(!registry.is_variant("NonExistent", "Anything"));
}

#[test]
fn variant_index_and_payload() {
    let mut registry = EnumRegistry::new();
    registry.register(
        "core",
        "Result",
        vec![
            v_with_payload("Ok", Ty::int()),
            v_with_payload("Err", Ty::text()),
        ],
    );

    assert_eq!(registry.variant_index("Result", "Ok"), Some(0));
    assert_eq!(registry.variant_index("Result", "Err"), Some(1));
    assert_eq!(registry.variant_index("Result", "Maybe"), None);

    assert_eq!(registry.payload_ty("Result", "Ok"), Some(&Ty::int()));
    assert_eq!(registry.payload_ty("Result", "Err"), Some(&Ty::text()));
    assert_eq!(registry.payload_ty("Result", "Maybe"), None);
}

#[test]
fn unit_variant_has_no_payload() {
    let mut registry = EnumRegistry::new();
    registry.register(
        "core",
        "Optional",
        vec![v_with_payload("Some", Ty::int()), v("None")],
    );

    assert_eq!(registry.payload_ty("Optional", "Some"), Some(&Ty::int()));
    assert_eq!(registry.payload_ty("Optional", "None"), None);
}

// ── Testes de origin + ambiguous ──────────────────────

#[test]
fn merge_different_origins_marks_ambiguous() {
    let mut prelude = EnumRegistry::new();
    prelude.register_generic_with_defaults(
        "core",
        "Result",
        vec!["T".into(), "E".into()],
        vec![None, Some(Ty::text())],
        vec![
            v_with_payload("Ok", Ty::Var("T".into())),
            v_with_payload("Err", Ty::Var("E".into())),
        ],
    );

    let mut user = EnumRegistry::new();
    user.register(
        "user",
        "Result",
        vec![
            v_with_payload("Ok", Ty::int()),
            v_with_payload("Err", Ty::int()),
        ],
    );

    prelude.merge(user);

    // Result é ambíguo (definido em core + user)
    assert!(prelude.is_ambiguous("Result"));
    assert_eq!(prelude.origins_of("Result").len(), 2);

    // Unqualified lookup falha (ambíguo)
    assert!(!prelude.is_variant("Result", "Ok"));
    assert_eq!(prelude.payload_ty("Result", "Ok"), None);

    // Qualified lookup funciona
    assert!(prelude.is_variant_with_origin("core", "Result", "Ok"));
    assert!(prelude.is_variant_with_origin("user", "Result", "Ok"));
    assert_eq!(
        prelude.payload_ty_with_origin("core", "Result", "Err"),
        Some(&Ty::Var("E".into()))
    );
    assert_eq!(
        prelude.payload_ty_with_origin("user", "Result", "Err"),
        Some(&Ty::int())
    );
}

#[test]
fn merge_same_origin_overwrites() {
    let mut registry = EnumRegistry::new();
    registry.register("core", "Result", vec![v("Ok"), v("Err")]);

    let mut update = EnumRegistry::new();
    update.register("core", "Result", vec![v("Success"), v("Failure")]);

    registry.merge(update);

    // Same origin — overwritten, not ambiguous
    assert!(!registry.is_ambiguous("Result"));
    assert!(registry.is_variant("Result", "Success"));
    assert!(!registry.is_variant("Result", "Ok"));
}

#[test]
fn resolve_origin_single() {
    let mut registry = EnumRegistry::new();
    registry.register("core", "Boolean", vec![v("True")]);

    assert_eq!(registry.resolve_origin("Boolean"), Some("core"));
    assert_eq!(registry.resolve_origin("NonExistent"), None);
}

#[test]
fn resolve_origin_ambiguous() {
    let mut registry = EnumRegistry::new();
    registry.register("core", "Result", vec![v("Ok")]);
    registry.register("user", "Result", vec![v("Err")]);

    // Ambiguous — resolve_origin returns None
    assert_eq!(registry.resolve_origin("Result"), None);
    assert!(registry.is_ambiguous("Result"));
}
