use super::*;

fn entry(origin: &str, type_name: &str, iface: &str) -> RefinesEntry {
    RefinesEntry {
        origin: origin.into(),
        type_name: type_name.into(),
        base_ty: Ty::int(),
        interface_name: iface.into(),
    }
}

#[test]
fn register_and_query() {
    let mut reg = RefinesRegistry::new();
    reg.register(entry("user", "PositiveInt", "NUM"));

    assert!(reg.has_refines("PositiveInt"));
    let delegations = reg.get("PositiveInt");
    assert_eq!(delegations.len(), 1);
    assert_eq!(delegations[0].interface_name, "NUM");
}

#[test]
fn no_refines_returns_empty() {
    let reg = RefinesRegistry::new();
    assert!(!reg.has_refines("Unknown"));
    assert!(reg.get("Unknown").is_empty());
}

#[test]
fn interfaces_of() {
    let mut reg = RefinesRegistry::new();
    reg.register(entry("user", "PositiveInt", "NUM"));
    reg.register(entry("user", "PositiveInt", "SHOW"));

    let ifaces = reg.interfaces_of("PositiveInt");
    assert_eq!(ifaces, vec!["NUM", "SHOW"]);
}

#[test]
fn merge_different_origins_marks_ambiguous() {
    let mut a = RefinesRegistry::new();
    a.register(entry("core", "PositiveInt", "NUM"));

    let mut b = RefinesRegistry::new();
    b.register(entry("user", "PositiveInt", "SHOW"));

    a.merge(b);
    assert!(a.is_ambiguous("PositiveInt"));
    assert!(a.resolve_origin("PositiveInt").is_none());
    assert!(a.get("PositiveInt").is_empty()); // ambíguo → vazio
}

#[test]
fn merge_same_origin_overwrites() {
    let mut a = RefinesRegistry::new();
    a.register(entry("core", "PositiveInt", "NUM"));

    let mut b = RefinesRegistry::new();
    b.register(entry("core", "PositiveInt", "SHOW"));

    a.merge(b);
    assert!(!a.is_ambiguous("PositiveInt"));
    // Mesma origin → insert sobrescreve, delegações de b substituem a
    let delegations = a.get("PositiveInt");
    assert_eq!(delegations.len(), 1);
    assert_eq!(delegations[0].interface_name, "SHOW");
}

#[test]
fn get_with_origin_disambiguates() {
    let mut a = RefinesRegistry::new();
    a.register(entry("core", "PositiveInt", "NUM"));

    let mut b = RefinesRegistry::new();
    b.register(entry("user", "PositiveInt", "SHOW"));

    a.merge(b);

    let core = a.get_with_origin("core", "PositiveInt");
    assert_eq!(core.len(), 1);
    assert_eq!(core[0].interface_name, "NUM");

    let user = a.get_with_origin("user", "PositiveInt");
    assert_eq!(user.len(), 1);
    assert_eq!(user[0].interface_name, "SHOW");
}
