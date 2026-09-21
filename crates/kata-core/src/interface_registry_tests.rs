use super::*;
use crate::ty::PrimTy;

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
        span: kata_ast::Span::synthetic(),
        allows_incomplete: false,
        type_bounds: vec![],
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

// ── type_implements_generic (Fase 5) ─────────────────────────

fn impl_entry_generic(
    origin: &str,
    type_name: &str,
    iface_name: &str,
    type_bounds: Vec<(&str, &str)>,
) -> ImplEntry {
    ImplEntry {
        origin: origin.into(),
        type_name: type_name.into(),
        type_params: vec![],
        interface_name: iface_name.into(),
        iface_params: vec![],
        methods: vec![],
        span: kata_ast::Span::synthetic(),
        allows_incomplete: false,
        type_bounds: type_bounds
            .into_iter()
            .map(|(n, b)| (n.to_string(), b.to_string()))
            .collect(),
    }
}

#[test]
fn type_implements_generic_with_bounds_satisfied() {
    let mut reg = InterfaceRegistry::new();
    // SCALAR extends NUM
    reg.register_interface("core", iface("NUM", &[])).unwrap();
    reg.register_interface("core", iface("SCALAR", &["NUM"]))
        .unwrap();
    reg.register_interface("core", iface("RING", &["NUM"]))
        .unwrap();

    // Int implements SCALAR (e herda NUM)
    reg.register_impl(impl_entry("core", "Int", "SCALAR"))
        .unwrap();
    // Float implements SCALAR
    reg.register_impl(impl_entry("core", "Float", "SCALAR"))
        .unwrap();
    // Complex implements RING, com type_bounds: T → SCALAR
    reg.register_impl(impl_entry_generic(
        "user",
        "Complex",
        "RING",
        vec![("T", "SCALAR")],
    ))
    .unwrap();

    // Complex::(Int, Int) implements RING? → Int satisfaz SCALAR → true
    assert!(reg.type_implements_generic(
        "Complex",
        &[Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Int)],
        "RING"
    ));

    // Complex::(Float, Float) implements RING? → Float satisfaz SCALAR → true
    assert!(reg.type_implements_generic(
        "Complex",
        &[Ty::Prim(PrimTy::Float), Ty::Prim(PrimTy::Float)],
        "RING"
    ));
}

#[test]
fn type_implements_generic_with_bounds_violated() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("NUM", &[])).unwrap();
    reg.register_interface("core", iface("SCALAR", &["NUM"]))
        .unwrap();
    reg.register_interface("core", iface("RING", &["NUM"]))
        .unwrap();

    // Int implements SCALAR, Text NÃO
    reg.register_impl(impl_entry("core", "Int", "SCALAR"))
        .unwrap();
    reg.register_impl(impl_entry_generic(
        "user",
        "Complex",
        "RING",
        vec![("T", "SCALAR")],
    ))
    .unwrap();

    // Complex::(Text, Text) implements RING? → Text não satisfaz SCALAR → false
    assert!(!reg.type_implements_generic(
        "Complex",
        &[Ty::Prim(PrimTy::Text), Ty::Prim(PrimTy::Text)],
        "RING"
    ));
}

#[test]
fn type_implements_generic_no_bounds() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("RING", &[])).unwrap();

    // Complex implements RING sem type_bounds (tipo monomórfico)
    reg.register_impl(impl_entry("user", "Complex", "RING"))
        .unwrap();

    // Sem bounds, comporta como type_implements normal
    assert!(reg.type_implements_generic("Complex", &[], "RING"));
    assert!(!reg.type_implements_generic("Complex", &[], "NUM"));
}

#[test]
fn type_implements_generic_via_supertrait() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("core", iface("NUM", &[])).unwrap();
    reg.register_interface("core", iface("SCALAR", &["NUM"]))
        .unwrap();
    // RING extends SCALAR (que extends NUM)
    reg.register_interface("core", iface("RING", &["SCALAR"]))
        .unwrap();

    // Int implements NUM (não SCALAR diretamente, mas RING herda SCALAR)
    reg.register_impl(impl_entry("core", "Int", "NUM")).unwrap();
    // Complex implements RING com bound T → SCALAR
    reg.register_impl(impl_entry_generic(
        "user",
        "Complex",
        "RING",
        vec![("T", "SCALAR")],
    ))
    .unwrap();

    // Complex implementa RING diretamente → true (bounds satisfeitos via Int→SCALAR?
    // Int implementa NUM, mas SCALAR extends NUM — Int implementa SCALAR via herança?
    // type_implements verifica herança: Int→NUM, SCALAR herda NUM → iface_inherits(NUM, SCALAR)? Não!
    // iface_inherits percorre supertraits de NUM (vazio) — NUM não herda SCALAR.
    // A herança é SCALAR→NUM, não NUM→SCALAR. Int implementa NUM, não SCALAR.
    // Então Int NÃO implementa SCALAR → bound falha → false.
    assert!(!reg.type_implements_generic(
        "Complex",
        &[Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Int)],
        "RING"
    ));
}
