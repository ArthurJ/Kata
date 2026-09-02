use super::*;

fn iface(name: &str, supertraits: &[&str]) -> InterfaceInfo {
    InterfaceInfo {
        name: name.into(),
        supertraits: supertraits.iter().map(|s| s.to_string()).collect(),
        type_params: Vec::new(),
        signatures: Vec::new(),
    }
}

fn impl_entry(origin: &str, type_name: &str, iface_name: &str) -> ImplEntry {
    ImplEntry {
        origin: origin.into(),
        type_name: type_name.into(),
        type_params: Vec::new(),
        interface_name: iface_name.into(),
        iface_params: Vec::new(),
        methods: Vec::new(),
    }
}

#[test]
fn register_and_query_interface() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("EQ", &[])).unwrap();
    reg.register_interface("core", iface("ORD", &["EQ"]))
        .unwrap();

    assert!(reg.get_interface("EQ").is_some());
    assert!(reg.get_interface("ORD").is_some());
    assert!(reg.get_interface("NUM").is_none());
}

#[test]
fn duplicate_interface_same_origin_is_error() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("EQ", &[])).unwrap();
    let err = reg.register_interface("core", iface("EQ", &[]));
    assert!(err.is_err());
}

#[test]
fn duplicate_interface_different_origin_coexists() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("EQ", &[])).unwrap();
    let result = reg.register_interface("user", iface("EQ", &[]));
    assert!(result.is_ok());
    assert!(reg.is_ambiguous("EQ"));
    assert!(reg.resolve_origin("EQ").is_none());
    assert!(reg.get_interface("EQ").is_none()); // ambíguo
    assert!(reg.get_interface_with_origin("core", "EQ").is_some());
    assert!(reg.get_interface_with_origin("user", "EQ").is_some());
}

#[test]
fn cycle_detection() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("A", &["B"])).unwrap();
    let err = reg.register_interface("core", iface("B", &["A"]));
    assert!(err.is_err());
}

#[test]
fn register_impl_accepts_unknown_interface() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("NUM", &["ORD"]))
        .unwrap();
    reg.register_impl(impl_entry("user", "Int", "NUM")).unwrap();

    let result = reg.register_impl(impl_entry("user", "Int", "SHOW"));
    assert!(result.is_ok());
    assert_eq!(reg.get_impls_for_interface("SHOW").len(), 1);
}

#[test]
fn register_impl_rejects_duplicate_same_origin() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("NUM", &[])).unwrap();
    reg.register_impl(impl_entry("user", "Int", "NUM")).unwrap();
    let err = reg.register_impl(impl_entry("user", "Int", "NUM"));
    assert!(err.is_err());
}

#[test]
fn register_impl_allows_same_impl_different_origin() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("NUM", &[])).unwrap();
    reg.register_impl(impl_entry("core", "Int", "NUM")).unwrap();
    let result = reg.register_impl(impl_entry("user", "Int", "NUM"));
    assert!(result.is_ok());
}

#[test]
fn type_implements_direct() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("NUM", &["ORD"]))
        .unwrap();
    reg.register_interface("core", iface("ORD", &["EQ"]))
        .unwrap();
    reg.register_interface("core", iface("EQ", &[])).unwrap();
    reg.register_impl(impl_entry("user", "Int", "NUM")).unwrap();

    assert!(reg.type_implements("Int", "NUM"));
    assert!(reg.type_implements("Int", "ORD"));
    assert!(reg.type_implements("Int", "EQ"));
    assert!(!reg.type_implements("Int", "SHOW"));
    assert!(!reg.type_implements("Float", "NUM"));
}

#[test]
fn get_impls_for_type_and_interface() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("NUM", &[])).unwrap();
    reg.register_interface("core", iface("SHOW", &[])).unwrap();
    reg.register_impl(impl_entry("user", "Int", "NUM")).unwrap();
    reg.register_impl(impl_entry("user", "Int", "SHOW"))
        .unwrap();
    reg.register_impl(impl_entry("user", "Float", "NUM"))
        .unwrap();

    assert_eq!(reg.get_impls_for_type("Int").len(), 2);
    assert_eq!(reg.get_impls_for_type("Float").len(), 1);
    assert_eq!(reg.get_impls_for_interface("NUM").len(), 2);
    assert_eq!(reg.get_impls_for_interface("SHOW").len(), 1);
}

#[test]
fn merge_two_registries() {
    let mut a = InterfaceRegistry::new();
    a.register_interface("core", iface("EQ", &[])).unwrap();

    let mut b = InterfaceRegistry::new();
    b.register_interface("core", iface("NUM", &["ORD"]))
        .unwrap();
    b.register_impl(impl_entry("user", "Int", "NUM")).unwrap();

    a.merge(b);
    assert!(a.get_interface("EQ").is_some());
    assert!(a.get_interface("NUM").is_some());
    assert!(a.type_implements("Int", "NUM"));
}

#[test]
fn merge_different_origins_marks_ambiguous() {
    let mut a = InterfaceRegistry::new();
    a.register_interface("core", iface("EQ", &[])).unwrap();

    let mut b = InterfaceRegistry::new();
    b.register_interface("user", iface("EQ", &[])).unwrap();

    a.merge(b);
    assert!(a.is_ambiguous("EQ"));
    assert!(a.resolve_origin("EQ").is_none());
    assert!(a.get_interface("EQ").is_none());
}
