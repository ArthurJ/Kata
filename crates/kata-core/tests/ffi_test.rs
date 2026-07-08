use kata_core::{FfiSymbol, Ty};

#[test]
fn ffi_symbol_name_bi_add() {
    assert_eq!(FfiSymbol::BiAdd.symbol_name(), "kata_rt_bi_add");
}

#[test]
fn ffi_symbol_name_print() {
    assert_eq!(FfiSymbol::Print.symbol_name(), "kata_rt_print");
}

#[test]
fn ffi_return_type_arithmetic() {
    assert_eq!(FfiSymbol::BiAdd.return_type(), Ty::int());
    assert_eq!(FfiSymbol::Fadd.return_type(), Ty::float());
    assert_eq!(FfiSymbol::RatAdd.return_type(), Ty::rational());
}

#[test]
fn ffi_return_type_comparison() {
    assert_eq!(FfiSymbol::BiEq.return_type(), Ty::boolean());
    assert_eq!(FfiSymbol::FcmpLt.return_type(), Ty::boolean());
}

#[test]
fn ffi_return_type_io() {
    assert_eq!(FfiSymbol::Print.return_type(), Ty::Unit);
}

#[test]
fn ffi_from_name_roundtrip() {
    let symbol = FfiSymbol::BiAdd;
    let name = symbol.symbol_name();
    assert_eq!(FfiSymbol::from_name(name), Some(symbol));
}

#[test]
fn ffi_from_name_unknown() {
    assert_eq!(FfiSymbol::from_name("kata_rt_nonexistent"), None);
}

#[test]
fn ffi_from_name_all_symbols_roundtrip() {
    let all = [
        FfiSymbol::BiAdd,
        FfiSymbol::BiSub,
        FfiSymbol::BiMul,
        FfiSymbol::BiDiv,
        FfiSymbol::BiEq,
        FfiSymbol::BiLt,
        FfiSymbol::BiGt,
        FfiSymbol::Fadd,
        FfiSymbol::Fsub,
        FfiSymbol::Fmul,
        FfiSymbol::Fdiv,
        FfiSymbol::FcmpEq,
        FfiSymbol::FcmpLt,
        FfiSymbol::RatAdd,
        FfiSymbol::RatSub,
        FfiSymbol::RatMul,
        FfiSymbol::RatDiv,
        FfiSymbol::RatEq,
        FfiSymbol::Print,
        FfiSymbol::Println,
    ];
    for s in all {
        let name = s.symbol_name();
        assert_eq!(
            FfiSymbol::from_name(name),
            Some(s),
            "roundtrip failed for {name}"
        );
    }
}
