//! Testes E2E de dispatch com interfaces (iface++ no Score).
//!
//! Verifica que match_score pontua iface++ quando o argumento implementa
//! a interface esperada, e que o dispatch seleciona a overload correta.

use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::interface_registry::{ImplEntry, InterfaceInfo, InterfaceRegistry};
use kata_core::ty::Ty;

fn make_ffi_info(name: &str, params: &[Ty], ret: Ty, ffi: &str) -> OverloadInfo {
    OverloadInfo {
        name: name.to_string(),
        params: params.to_vec(),
        ret,
        ffi_symbol: Some(ffi.to_string()),
        is_action: false,
        is_generic: false,
        is_constructor: false,
        associative_neutral: None,
        type_params: vec![],
        substitutions: None,
        param_names: vec![],
    }
}

fn make_iface_info(name: &str, supertraits: &[&str]) -> InterfaceInfo {
    InterfaceInfo {
        name: name.into(),
        supertraits: supertraits.iter().map(|s| s.to_string()).collect(),
        type_params: Vec::new(),
        signatures: Vec::new(),
    }
}

fn make_impl_entry(type_name: &str, iface_name: &str) -> ImplEntry {
    ImplEntry {
        origin: "test".to_string(),
        type_name: type_name.into(),
        type_params: Vec::new(),
        interface_name: iface_name.into(),
        iface_params: Vec::new(),
        methods: Vec::new(),
    }
}

/// iface++ básico: overload com `Ty::Interface("NUM")` como parâmetro
/// despacha quando o arg é `Int` (que implementa NUM).
#[test]
fn iface_dispatch_int_implements_num() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("test", make_iface_info("NUM", &["ORD"]))
        .unwrap();
    reg.register_interface("test", make_iface_info("ORD", &["EQ"]))
        .unwrap();
    reg.register_interface("test", make_iface_info("EQ", &[]))
        .unwrap();
    reg.register_impl(make_impl_entry("Int", "NUM")).unwrap();

    let mut table = DispatchTable::new();
    // + :: NUM NUM => NUM
    table.insert(make_ffi_info(
        "+",
        &[Ty::Interface("NUM".into()), Ty::Interface("NUM".into())],
        Ty::Interface("NUM".into()),
        "kata_rt_add",
    ));

    // Args Int Int → Int implementa NUM → iface++ → compatível
    let result = table.resolve("+", &[Ty::int(), Ty::int()], &reg);
    assert!(
        result.is_ok(),
        "dispatch deveria encontrar overload via iface++"
    );
    assert_eq!(result.unwrap().ffi_symbol.as_deref(), Some("kata_rt_add"));
}

/// iface++ com supertrait: Int implementa NUM, NUM : ORD → Int implementa ORD.
/// Overload com `Ty::Interface("ORD")` despacha para arg Int.
#[test]
fn iface_dispatch_via_supertrait() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("test", make_iface_info("NUM", &["ORD"]))
        .unwrap();
    reg.register_interface("test", make_iface_info("ORD", &["EQ"]))
        .unwrap();
    reg.register_interface("test", make_iface_info("EQ", &[]))
        .unwrap();
    reg.register_impl(make_impl_entry("Int", "NUM")).unwrap();

    let mut table = DispatchTable::new();
    // < :: ORD ORD => Boolean
    table.insert(make_ffi_info(
        "<",
        &[Ty::Interface("ORD".into()), Ty::Interface("ORD".into())],
        Ty::boolean(),
        "kata_rt_lt",
    ));

    // Int implementa NUM, NUM : ORD → Int implementa ORD via supertrait
    let result = table.resolve("<", &[Ty::int(), Ty::int()], &reg);
    assert!(
        result.is_ok(),
        "dispatch deveria encontrar overload via supertrait"
    );
}

/// Dispatch scoring: exact vence iface.
/// Overload `+ :: Int Int => Int` (exact) e `+ :: NUM NUM => NUM` (iface).
/// Args Int Int → exact vence iface.
#[test]
fn exact_beats_iface() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("test", make_iface_info("NUM", &[]))
        .unwrap();
    reg.register_impl(make_impl_entry("Int", "NUM")).unwrap();

    let mut table = DispatchTable::new();
    // + :: Int Int => Int (exact match)
    table.insert(make_ffi_info(
        "+",
        &[Ty::int(), Ty::int()],
        Ty::int(),
        "kata_rt_bi_add",
    ));
    // + :: NUM NUM => NUM (iface match)
    table.insert(make_ffi_info(
        "+",
        &[Ty::Interface("NUM".into()), Ty::Interface("NUM".into())],
        Ty::Interface("NUM".into()),
        "kata_rt_add_generic",
    ));

    // Args Int Int → ambas compatíveis, mas exact > iface
    let result = table.resolve("+", &[Ty::int(), Ty::int()], &reg);
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().ffi_symbol.as_deref(),
        Some("kata_rt_bi_add"),
        "exact deve vencer iface"
    );
}

/// Dispatch scoring: iface vence incompatível.
/// Overload `+ :: NUM NUM => NUM` e `+ :: Float Float => Float`.
/// Args Int Int → Float é incompatível, NUM é compatível via iface++.
#[test]
fn iface_beats_incompatible() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("test", make_iface_info("NUM", &[]))
        .unwrap();
    reg.register_impl(make_impl_entry("Int", "NUM")).unwrap();

    let mut table = DispatchTable::new();
    // + :: Float Float => Float (incompatível com Int)
    table.insert(make_ffi_info(
        "+",
        &[Ty::float(), Ty::float()],
        Ty::float(),
        "kata_rt_fadd",
    ));
    // + :: NUM NUM => NUM (iface compatível com Int)
    table.insert(make_ffi_info(
        "+",
        &[Ty::Interface("NUM".into()), Ty::Interface("NUM".into())],
        Ty::Interface("NUM".into()),
        "kata_rt_add_generic",
    ));

    // Args Int Int → só NUM é compatível
    let result = table.resolve("+", &[Ty::int(), Ty::int()], &reg);
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().ffi_symbol.as_deref(),
        Some("kata_rt_add_generic"),
        "iface deve vencer incompatível"
    );
}

/// Tipo que não implementa a interface → dispatch falha.
#[test]
fn iface_dispatch_fails_when_not_implemented() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("test", make_iface_info("NUM", &[]))
        .unwrap();
    reg.register_impl(make_impl_entry("Int", "NUM")).unwrap();
    // Float NÃO implementa NUM

    let mut table = DispatchTable::new();
    // + :: NUM NUM => NUM
    table.insert(make_ffi_info(
        "+",
        &[Ty::Interface("NUM".into()), Ty::Interface("NUM".into())],
        Ty::Interface("NUM".into()),
        "kata_rt_add_generic",
    ));

    // Args Float Float → Float não implementa NUM → incompatível
    let result = table.resolve("+", &[Ty::float(), Ty::float()], &reg);
    assert!(
        result.is_err(),
        "dispatch deveria falhar: Float não implementa NUM"
    );
}

/// iface++ com tipo do usuário (Struct): Complex implementa NUM.
#[test]
fn iface_dispatch_user_type() {
    let mut reg = InterfaceRegistry::new();
    reg.register_interface("test", make_iface_info("NUM", &[]))
        .unwrap();
    reg.register_impl(make_impl_entry("Complex", "NUM"))
        .unwrap();

    let mut table = DispatchTable::new();
    // + :: NUM NUM => NUM
    table.insert(make_ffi_info(
        "+",
        &[Ty::Interface("NUM".into()), Ty::Interface("NUM".into())],
        Ty::Interface("NUM".into()),
        "kata_rt_complex_add",
    ));

    // Complex implementa NUM → iface++
    let complex = Ty::Struct("Complex".into());
    let result = table.resolve("+", &[complex.clone(), complex], &reg);
    assert!(
        result.is_ok(),
        "dispatch deveria encontrar overload para Complex via iface++"
    );
}

/// match_score direto: arg Int, param Ty::Interface("NUM") → iface=1
#[test]
fn match_score_iface_basic() {
    use kata_core::dispatch::match_score;

    let mut reg = InterfaceRegistry::new();
    reg.register_interface("test", make_iface_info("NUM", &[]))
        .unwrap();
    reg.register_impl(make_impl_entry("Int", "NUM")).unwrap();

    let score = match_score(
        &[Ty::int(), Ty::int()],
        &[Ty::Interface("NUM".into()), Ty::Interface("NUM".into())],
        &reg,
    );
    assert!(score.is_compatible(2), "ambos args devem casar via iface");
    assert_eq!(score.exact, 0);
    assert_eq!(score.iface, 2);
}

/// match_score misto: arg Int, param Int → exact; arg Float, param NUM → iface
/// (se Float implementa NUM)
#[test]
fn match_score_mixed_exact_iface() {
    use kata_core::dispatch::match_score;

    let mut reg = InterfaceRegistry::new();
    reg.register_interface("test", make_iface_info("NUM", &[]))
        .unwrap();
    reg.register_impl(make_impl_entry("Int", "NUM")).unwrap();
    reg.register_impl(make_impl_entry("Float", "NUM")).unwrap();

    let score = match_score(
        &[Ty::int(), Ty::float()],
        &[Ty::int(), Ty::Interface("NUM".into())],
        &reg,
    );
    assert!(score.is_compatible(2));
    assert_eq!(score.exact, 1);
    assert_eq!(score.iface, 1);
}
