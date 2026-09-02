use super::*;

fn field(name: &str, ty: Ty, offset: u32) -> FieldInfo {
    FieldInfo {
        name: name.into(),
        ty,
        offset,
    }
}

#[test]
fn register_and_query() {
    let mut registry = StructRegistry::new();
    registry.register(
        "user",
        "Pessoa",
        vec![field("nome", Ty::text(), 0), field("idade", Ty::int(), 8)],
    );

    assert!(registry.contains("Pessoa"));
    assert!(!registry.contains("Inexistente"));

    let info = registry.get("Pessoa").unwrap();
    assert_eq!(info.num_fields(), 2);
    assert_eq!(info.size_bytes(), 16);
}

#[test]
fn find_field_by_name() {
    let mut registry = StructRegistry::new();
    registry.register(
        "user",
        "Pessoa",
        vec![field("nome", Ty::text(), 0), field("idade", Ty::int(), 8)],
    );

    let info = registry.get("Pessoa").unwrap();
    let (idx, f) = info.find_field("idade").unwrap();
    assert_eq!(idx, 1);
    assert_eq!(f.ty, Ty::int());
    assert_eq!(f.offset, 8);
}

#[test]
fn find_nonexistent_field_returns_none() {
    let mut registry = StructRegistry::new();
    registry.register("user", "Pessoa", vec![field("nome", Ty::text(), 0)]);

    let info = registry.get("Pessoa").unwrap();
    assert!(info.find_field("inexistente").is_none());
}

#[test]
fn field_types_in_order() {
    let mut registry = StructRegistry::new();
    registry.register(
        "user",
        "Pessoa",
        vec![field("nome", Ty::text(), 0), field("idade", Ty::int(), 8)],
    );

    let info = registry.get("Pessoa").unwrap();
    let types = info.field_types();
    assert_eq!(types, vec![&Ty::text(), &Ty::int()]);
}

#[test]
fn merge_two_registries() {
    let mut a = StructRegistry::new();
    a.register("user", "A", vec![field("x", Ty::int(), 0)]);

    let mut b = StructRegistry::new();
    b.register("user", "B", vec![field("y", Ty::text(), 0)]);

    a.merge(b);
    assert!(a.contains("A"));
    assert!(a.contains("B"));
}

#[test]
fn empty_registry_returns_none() {
    let registry = StructRegistry::new();
    assert!(registry.get("Qualquer").is_none());
    assert!(!registry.contains("Qualquer"));
}

#[test]
fn struct_with_zero_fields() {
    let mut registry = StructRegistry::new();
    registry.register("user", "Vazio", vec![]);

    let info = registry.get("Vazio").unwrap();
    assert_eq!(info.num_fields(), 0);
    assert_eq!(info.size_bytes(), 0);
}

// ── Testes de origin ──────────────────────────────────

#[test]
fn merge_different_origins_marks_ambiguous() {
    let mut a = StructRegistry::new();
    a.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

    let mut b = StructRegistry::new();
    b.register("user", "Pessoa", vec![field("nome", Ty::int(), 0)]);

    a.merge(b);
    assert!(a.is_ambiguous("Pessoa"));
    assert!(a.resolve_origin("Pessoa").is_none());
    assert!(a.get("Pessoa").is_none()); // ambíguo → None
}

#[test]
fn merge_same_origin_overwrites() {
    let mut a = StructRegistry::new();
    a.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

    let mut b = StructRegistry::new();
    b.register("core", "Pessoa", vec![field("nome", Ty::int(), 0)]);

    a.merge(b);
    assert!(!a.is_ambiguous("Pessoa"));
    assert_eq!(a.resolve_origin("Pessoa"), Some("core"));
    let info = a.get("Pessoa").unwrap();
    assert_eq!(info.fields[0].ty, Ty::int()); // sobrescreveu
}

#[test]
fn resolve_origin_single() {
    let mut registry = StructRegistry::new();
    registry.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

    assert_eq!(registry.resolve_origin("Pessoa"), Some("core"));
    assert!(!registry.is_ambiguous("Pessoa"));
}

#[test]
fn resolve_origin_ambiguous_returns_none() {
    let mut a = StructRegistry::new();
    a.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

    let mut b = StructRegistry::new();
    b.register("user", "Pessoa", vec![field("nome", Ty::int(), 0)]);

    a.merge(b);
    assert!(a.resolve_origin("Pessoa").is_none());
}

#[test]
fn get_with_origin_disambiguates() {
    let mut a = StructRegistry::new();
    a.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

    let mut b = StructRegistry::new();
    b.register("user", "Pessoa", vec![field("nome", Ty::int(), 0)]);

    a.merge(b);

    let core_info = a.get_with_origin("core", "Pessoa").unwrap();
    assert_eq!(core_info.fields[0].ty, Ty::text());

    let user_info = a.get_with_origin("user", "Pessoa").unwrap();
    assert_eq!(user_info.fields[0].ty, Ty::int());
}
