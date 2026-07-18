//! Resolution: interfaces e implements.
//!
//! Verifica que o resolution processa InterfaceDecl e ImplementsDecl,
//! registrando no InterfaceRegistry do ResolvedModule.

use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::resolve;

fn resolve_src(src: &str) -> kata_resolution::ResolvedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    resolve(&module).expect("resolve deve succeed")
}

#[test]
fn interface_decl_registra_no_registry() {
    let resolved = resolve_src("interface EQ\n    = :: Self Self => Boolean");
    let iface = resolved
        .interface_registry
        .get_interface("EQ")
        .expect("EQ deve estar no interface_registry");
    assert_eq!(iface.name, "EQ");
    assert!(iface.supertraits.is_empty());
    assert_eq!(iface.signatures.len(), 1);
    assert_eq!(iface.signatures[0].name, "=");
}

#[test]
fn interface_com_supertraits() {
    let src = "interface ORD implements EQ\n    < :: Self Self => Boolean";
    let resolved = resolve_src(src);
    let iface = resolved
        .interface_registry
        .get_interface("ORD")
        .expect("ORD deve estar no registry");
    assert_eq!(iface.supertraits, vec!["EQ"]);
}

#[test]
fn interface_com_type_params() {
    let src = "interface ITERABLE::(A)\n    next :: Self => Optional::(A)";
    let resolved = resolve_src(src);
    let iface = resolved
        .interface_registry
        .get_interface("ITERABLE")
        .expect("ITERABLE deve estar no registry");
    assert_eq!(iface.type_params, vec!["A"]);
}

#[test]
fn implements_decl_registra_no_registry() {
    let src = "\
interface NUM\n    + :: NUM NUM => NUM
Int implements NUM\n    + :: Int Int => Int @ffi(\"kata_rt_bi_add\")";
    let resolved = resolve_src(src);
    assert!(resolved.interface_registry.type_implements("Int", "NUM"));
}

#[test]
fn implements_com_type_params() {
    let src = "\
interface ITERABLE::(A)\n    next :: Self => Optional::(A)
List implements ITERABLE::(A)\n    next :: List => Optional::(A)";
    let resolved = resolve_src(src);
    let impls = resolved.interface_registry.get_impls_for_type("List");
    assert_eq!(impls.len(), 1);
    assert_eq!(impls[0].interface_name, "ITERABLE");
    assert_eq!(impls[0].iface_params, vec!["A"]);
}

#[test]
fn supertrait_propaga_type_implements() {
    let src = "\
interface EQ\n    = :: Self Self => Boolean
interface ORD implements EQ\n    < :: Self Self => Boolean
interface NUM implements ORD EQ\n    + :: NUM NUM => NUM
Int implements NUM\n    + :: Int Int => Int";
    let resolved = resolve_src(src);
    // Int implementa NUM diretamente
    assert!(resolved.interface_registry.type_implements("Int", "NUM"));
    // Via supertrait: NUM : ORD → Int implementa ORD
    assert!(resolved.interface_registry.type_implements("Int", "ORD"));
    // Via supertrait em cadeia: NUM : ORD : EQ → Int implementa EQ
    assert!(resolved.interface_registry.type_implements("Int", "EQ"));
}

#[test]
fn metodo_com_ffi_symbol_registrado() {
    let src = "\
interface NUM\n    + :: NUM NUM => NUM
Int implements NUM\n    + :: Int Int => Int @ffi(\"kata_rt_bi_add\")";
    let resolved = resolve_src(src);
    let impls = resolved.interface_registry.get_impls_for_type("Int");
    assert_eq!(impls.len(), 1);
    let method = &impls[0].methods[0];
    assert_eq!(method.name, "+");
    assert_eq!(method.ffi_symbol.as_deref(), Some("kata_rt_bi_add"));
}

#[test]
fn metodo_sem_ffi_tem_symbol_none() {
    let src = "\
interface NUM\n    + :: NUM NUM => NUM
Complex implements NUM\n    + :: Complex Complex => Complex";
    let resolved = resolve_src(src);
    let impls = resolved.interface_registry.get_impls_for_type("Complex");
    let method = &impls[0].methods[0];
    assert!(method.ffi_symbol.is_none());
}
