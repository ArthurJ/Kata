//! Fio 6 — Resolution: refined types e enum predicados.

use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::resolve;

fn resolve_src(src: &str) -> kata_resolution::ResolvedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    resolve(&module).expect("resolve deve succeed")
}

#[test]
fn refined_decl_registra_no_struct_registry() {
    let resolved = resolve_src("data (Int, > _ 0) as PositiveInt");
    let info = resolved
        .struct_registry
        .get("PositiveInt")
        .expect("PositiveInt deve estar no struct_registry");
    assert_eq!(info.name, "PositiveInt");
    assert!(info.fields.is_empty());
    assert_eq!(info.alias_of.as_deref(), Some("Int"));
    let preds = info.predicates.as_ref().expect("deve ter predicates");
    assert_eq!(preds.len(), 1);
    assert_eq!(preds[0], "__pred_PositiveInt_0");
}

#[test]
fn refined_decl_popula_refined_decls() {
    let resolved = resolve_src("data (Int, > _ 0) as PositiveInt");
    assert_eq!(resolved.refined_decls.len(), 1);
    let r = &resolved.refined_decls[0];
    assert_eq!(r.name, "PositiveInt");
    assert_eq!(r.base_ty, kata_core::Ty::Prim(kata_core::PrimTy::Int));
    assert_eq!(r.predicates.len(), 1);
}

#[test]
fn refined_decl_multiple_predicates() {
    let resolved = resolve_src("data (Int, > _ 0, <= _ 100) as Percentage");
    let info = resolved.struct_registry.get("Percentage").unwrap();
    let preds = info.predicates.as_ref().unwrap();
    assert_eq!(preds.len(), 2);
    assert_eq!(preds[0], "__pred_Percentage_0");
    assert_eq!(preds[1], "__pred_Percentage_1");

    assert_eq!(resolved.refined_decls.len(), 1);
    assert_eq!(resolved.refined_decls[0].predicates.len(), 2);
}

#[test]
fn refined_decl_float_base() {
    let resolved = resolve_src("data (Float, >= _ 0.0) as NonNegFloat");
    let info = resolved.struct_registry.get("NonNegFloat").unwrap();
    assert_eq!(info.alias_of.as_deref(), Some("Float"));
}

#[test]
fn struct_normal_nao_tem_predicates() {
    let resolved = resolve_src("data Pessoa (nome::Text idade::Int)");
    let info = resolved.struct_registry.get("Pessoa").unwrap();
    assert!(info.predicates.is_none());
    assert!(resolved.refined_decls.is_empty());
}

#[test]
fn enum_predicado_nao_quebra_resolution() {
    // Apenas verifica que o enum com predicado resolve sem erro.
    // O registro do predicado como função será feito no inference.
    let resolved =
        resolve_src("enum IMC\n    Magreza(< _ 18.5)\n    Normal(<= _ 25.0)\n    Obesidade");
    assert_eq!(
        resolved.enum_registry.variants_of("IMC"),
        vec!["Magreza", "Normal", "Obesidade"]
    );
}
