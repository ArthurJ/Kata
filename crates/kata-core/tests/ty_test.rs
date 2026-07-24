use kata_core::{PrimTy, Ty, TypeEnv};

#[test]
fn ty_int_convenience() {
    assert_eq!(Ty::int(), Ty::Prim(PrimTy::Int));
}

#[test]
fn ty_float_convenience() {
    assert_eq!(Ty::float(), Ty::Prim(PrimTy::Float));
}

#[test]
fn ty_text_convenience() {
    assert_eq!(Ty::text(), Ty::Prim(PrimTy::Text));
}

#[test]
fn ty_rational_convenience() {
    assert_eq!(Ty::rational(), Ty::Prim(PrimTy::Rational));
}

#[test]
fn ty_boolean_is_sum() {
    assert_eq!(Ty::boolean(), Ty::Sum("Boolean".into()));
}

#[test]
fn primty_ffi_repr() {
    assert_eq!(PrimTy::Int.ffi_repr(), "i64");
    assert_eq!(PrimTy::Float.ffi_repr(), "f64");
    assert_eq!(PrimTy::Text.ffi_repr(), "kata_rt_string");
    assert_eq!(PrimTy::Rational.ffi_repr(), "kata_rt_rat");
}

#[test]
fn primty_from_ffi() {
    assert_eq!(PrimTy::from_ffi("i64"), Some(PrimTy::Int));
    assert_eq!(PrimTy::from_ffi("f64"), Some(PrimTy::Float));
    assert_eq!(PrimTy::from_ffi("kata_rt_string"), Some(PrimTy::Text));
    assert_eq!(PrimTy::from_ffi("kata_rt_rat"), Some(PrimTy::Rational));
    assert_eq!(PrimTy::from_ffi("unknown"), None);
}

#[test]
fn typeenv_define_and_lookup() {
    let mut env = TypeEnv::new();
    env.define("x", Ty::int(), "test");
    assert_eq!(env.lookup("x"), Some(&Ty::int()));
}

#[test]
fn typeenv_lookup_missing_returns_none() {
    let env = TypeEnv::new();
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn typeenv_push_scope_inherits_parent() {
    let mut parent = TypeEnv::new();
    parent.define("x", Ty::int(), "test");
    let child = parent.push_scope();
    assert_eq!(child.lookup("x"), Some(&Ty::int()));
}

#[test]
fn typeenv_push_scope_can_shadow() {
    let mut parent = TypeEnv::new();
    parent.define("x", Ty::int(), "test");
    let mut child = parent.push_scope();
    child.define("x", Ty::float(), "test");
    assert_eq!(child.lookup("x"), Some(&Ty::float()));
}

#[test]
fn typeenv_grandchild_sees_grandparent() {
    let mut parent = TypeEnv::new();
    parent.define("z", Ty::text(), "test");
    let child = parent.push_scope();
    let grandchild = child.push_scope();
    assert_eq!(grandchild.lookup("z"), Some(&Ty::text()));
}
