use kata_core::{PrimTy, Ty};
use kata_resolution::load_prelude;

#[test]
fn prelude_has_int_type() {
    let resolved = load_prelude().expect("prelude deve resolver");
    assert_eq!(
        resolved.type_env.lookup("Int"),
        Some(&Ty::Prim(PrimTy::Int))
    );
}

#[test]
fn prelude_has_float_type() {
    let resolved = load_prelude().expect("prelude deve resolver");
    assert_eq!(
        resolved.type_env.lookup("Float"),
        Some(&Ty::Prim(PrimTy::Float))
    );
}

#[test]
fn prelude_has_text_type() {
    let resolved = load_prelude().expect("prelude deve resolver");
    assert_eq!(
        resolved.type_env.lookup("Text"),
        Some(&Ty::Prim(PrimTy::Text))
    );
}

#[test]
fn prelude_has_rational_type() {
    let resolved = load_prelude().expect("prelude deve resolver");
    assert_eq!(
        resolved.type_env.lookup("Rational"),
        Some(&Ty::Prim(PrimTy::Rational))
    );
}

#[test]
fn prelude_has_boolean_type() {
    let resolved = load_prelude().expect("prelude deve resolver");
    assert_eq!(
        resolved.type_env.lookup("Boolean"),
        Some(&Ty::Sum("Boolean".into()))
    );
}

#[test]
fn prelude_has_unit_type() {
    let resolved = load_prelude().expect("prelude deve resolver");
    assert_eq!(resolved.type_env.lookup("Unit"), Some(&Ty::Unit));
}

#[test]
fn prelude_has_int_add_signature() {
    let resolved = load_prelude().expect("prelude deve resolver");
    let add = resolved
        .signatures
        .iter()
        .find(|s| s.name == "+" && s.param_types == vec![Ty::int(), Ty::int()])
        .expect("deve ter + :: Int Int => Int");
    assert_eq!(add.return_type, Ty::int());
    assert_eq!(add.ffi_symbol.as_deref(), Some("kata_rt_bi_add"));
    assert!(add.is_associative);
    assert_eq!(add.associative_neutral, Some(0));
}

#[test]
fn prelude_has_float_add_signature() {
    let resolved = load_prelude().expect("prelude deve resolver");
    let add = resolved
        .signatures
        .iter()
        .find(|s| s.name == "+" && s.param_types == vec![Ty::float(), Ty::float()])
        .expect("deve ter + :: Float Float => Float");
    assert_eq!(add.return_type, Ty::float());
    assert_eq!(add.ffi_symbol.as_deref(), Some("kata_rt_fadd"));
}

#[test]
fn prelude_has_rational_add_signature() {
    let resolved = load_prelude().expect("prelude deve resolver");
    let add = resolved
        .signatures
        .iter()
        .find(|s| s.name == "+" && s.param_types == vec![Ty::rational(), Ty::rational()])
        .expect("deve ter + :: Rational Rational => Rational");
    assert_eq!(add.return_type, Ty::rational());
    assert!(add.is_associative);
    assert_eq!(add.associative_neutral, Some(0));
}

#[test]
fn prelude_has_echo_signature() {
    let resolved = load_prelude().expect("prelude deve resolver");
    // echo agora é uma Action Kata (não FFI) com body que despacha show.
    // Vai para resolved.actions, não resolved.signatures.
    let echo = resolved
        .actions
        .iter()
        .find(|a| a.name == "echo")
        .expect("deve ter echo :: SHOW => Unit em actions");
    assert_eq!(echo.param_types, vec![Ty::Interface("SHOW".into())]);
    assert_eq!(echo.return_type, Ty::Unit);
}

#[test]
fn prelude_has_multiple_add_overloads() {
    let resolved = load_prelude().expect("prelude deve resolver");
    let adds: Vec<_> = resolved
        .signatures
        .iter()
        .filter(|s| s.name == "+")
        .collect();
    assert_eq!(
        adds.len(),
        12,
        "deve ter 12 overloads de + (Int, Float, Rational, List, Set+Set, Set+elem, Dict+Dict, Bytes+Bytes + 4 cross-type: Int Float, Int Rational, Float Rational, Rational Float)"
    );
}

#[test]
fn prelude_has_show_for_int_float_rational_and_text() {
    let resolved = load_prelude().expect("prelude deve resolver");
    let shows: Vec<_> = resolved
        .signatures
        .iter()
        .filter(|s| s.name == "show")
        .collect();
    assert_eq!(
        shows.len(),
        5,
        "deve ter 5 overloads de show (Int, Float, Rational, Text, Bytes)"
    );
}
