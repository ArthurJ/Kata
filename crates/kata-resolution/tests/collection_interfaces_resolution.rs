//! Resolution — interfaces de coleção no prelude.
//!
//! Verifica que:
//! - As 4 interfaces (ITERABLE, COUNTABLE, INDEXABLE, CONTAINS) estão registradas
//! - Array, List, Range, Text implementam as interfaces apropriadas
//! - As signatures dos métodos chegam ao ResolvedModule (DispatchTable)
//! - @builtin é extraído como símbolo (não apenas @ffi)
//! - resolve_type_expr mapeia List::(A) → Ty::List(A), etc.

use kata_core::Ty;
use kata_resolution::load_stdlib_for_tests;

/// Carrega o prelude e verifica as 4 interfaces de coleção.
#[test]
fn prelude_has_collection_interfaces() {
    let resolved = load_stdlib_for_tests().expect("prelude deve resolver");

    // DoD 17: ITERABLE(A), COUNTABLE, INDEXABLE(A), CONTAINS(A) registradas
    assert!(
        resolved
            .interface_registry
            .get_interface("ITERABLE")
            .is_some(),
        "ITERABLE deve estar no InterfaceRegistry"
    );
    assert!(
        resolved
            .interface_registry
            .get_interface("COUNTABLE")
            .is_some(),
        "COUNTABLE deve estar no InterfaceRegistry"
    );
    assert!(
        resolved
            .interface_registry
            .get_interface("INDEXABLE")
            .is_some(),
        "INDEXABLE deve estar no InterfaceRegistry"
    );
    assert!(
        resolved
            .interface_registry
            .get_interface("CONTAINS")
            .is_some(),
        "CONTAINS deve estar no InterfaceRegistry"
    );

    // Verifica type_params das interfaces
    let iterable = resolved
        .interface_registry
        .get_interface("ITERABLE")
        .expect("ITERABLE");
    assert_eq!(iterable.type_params, vec!["A"]);

    let indexable = resolved
        .interface_registry
        .get_interface("INDEXABLE")
        .expect("INDEXABLE");
    assert_eq!(indexable.type_params, vec!["A"]);

    let contains = resolved
        .interface_registry
        .get_interface("CONTAINS")
        .expect("CONTAINS");
    assert_eq!(contains.type_params, vec!["A"]);

    let countable = resolved
        .interface_registry
        .get_interface("COUNTABLE")
        .expect("COUNTABLE");
    assert!(countable.type_params.is_empty());
}

// ── DoD 18: Array implements ITERABLE ──────────────────────────

#[test]
fn array_implements_iterable_countable_indexable_contains() {
    let resolved = load_stdlib_for_tests().expect("prelude deve resolver");

    assert!(
        resolved
            .interface_registry
            .type_implements("Array", "ITERABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("Array", "COUNTABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("Array", "INDEXABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("Array", "CONTAINS")
    );

    // Verifica que os métodos têm ffi_symbol (kata_rt_array_*)
    let impls = resolved.interface_registry.get_impls_for_type("Array");
    assert_eq!(
        impls.len(),
        5,
        "Array deve ter 5 implements entries (ITERABLE, COUNTABLE, INDEXABLE, CONTAINS, SLICEABLE)"
    );

    let iter_impl = impls
        .iter()
        .find(|i| i.interface_name == "ITERABLE")
        .expect("Array implements ITERABLE");
    let next_method = iter_impl
        .methods
        .iter()
        .find(|m| m.name == "next")
        .expect("next method");
    assert_eq!(
        next_method.ffi_symbol,
        Some("kata_rt_array_next".to_string()),
        "Array::next deve ter ffi_symbol kata_rt_array_next"
    );
}

// ── DoD 19: List implements ITERABLE ────────────────────────────

#[test]
fn list_implements_iterable_countable_indexable_contains() {
    let resolved = load_stdlib_for_tests().expect("prelude deve resolver");

    assert!(
        resolved
            .interface_registry
            .type_implements("List", "ITERABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("List", "COUNTABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("List", "INDEXABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("List", "CONTAINS")
    );

    let impls = resolved.interface_registry.get_impls_for_type("List");
    assert_eq!(
        impls.len(),
        5,
        "List deve ter 5 implements entries (ITERABLE, COUNTABLE, INDEXABLE, CONTAINS, SLICEABLE)"
    );

    let iter_impl = impls
        .iter()
        .find(|i| i.interface_name == "ITERABLE")
        .expect("List implements ITERABLE");
    let next_method = iter_impl
        .methods
        .iter()
        .find(|m| m.name == "next")
        .expect("next method");
    assert_eq!(
        next_method.ffi_symbol,
        Some("kata_rt_list_next".to_string()),
        "List::next deve ter ffi_symbol kata_rt_list_next"
    );
}

// ── DoD 20: Range implements ITERABLE ───────────────────────────

#[test]
fn range_implements_iterable_countable_contains_not_indexable() {
    let resolved = load_stdlib_for_tests().expect("prelude deve resolver");

    assert!(
        resolved
            .interface_registry
            .type_implements("Range", "ITERABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("Range", "COUNTABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("Range", "CONTAINS")
    );
    // Range NÃO implementa INDEXABLE
    assert!(
        !resolved
            .interface_registry
            .type_implements("Range", "INDEXABLE"),
        "Range não deve implementar INDEXABLE"
    );

    let impls = resolved.interface_registry.get_impls_for_type("Range");
    assert_eq!(
        impls.len(),
        3,
        "Range deve ter 3 implements entries (sem INDEXABLE)"
    );

    // Range usa @builtin (não @ffi) — ffi_symbol deve ser Some("range_next")
    let iter_impl = impls
        .iter()
        .find(|i| i.interface_name == "ITERABLE")
        .expect("Range implements ITERABLE");
    let next_method = iter_impl
        .methods
        .iter()
        .find(|m| m.name == "next")
        .expect("next method");
    assert_eq!(
        next_method.ffi_symbol,
        Some("range_next".to_string()),
        "Range::next deve ter símbolo builtin 'range_next'"
    );
}

// ── DoD 21: Text implements CONTAINS ─────────────────────────────

#[test]
fn text_implements_iterable_countable_indexable_contains() {
    let resolved = load_stdlib_for_tests().expect("prelude deve resolver");

    assert!(
        resolved
            .interface_registry
            .type_implements("Text", "ITERABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("Text", "COUNTABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("Text", "INDEXABLE")
    );
    assert!(
        resolved
            .interface_registry
            .type_implements("Text", "CONTAINS")
    );

    let impls = resolved.interface_registry.get_impls_for_type("Text");
    assert_eq!(
        impls.len(),
        7,
        "Text deve ter 7 implements entries (ITERABLE, COUNTABLE, INDEXABLE, CONTAINS, SHOW, HASHABLE, SLICEABLE)"
    );

    let contains_impl = impls
        .iter()
        .find(|i| i.interface_name == "CONTAINS")
        .expect("Text implements CONTAINS");
    let contains_method = contains_impl
        .methods
        .iter()
        .find(|m| m.name == "contains")
        .expect("contains method");
    assert_eq!(
        contains_method.ffi_symbol,
        Some("kata_rt_string_contains".to_string()),
        "Text::contains deve ter ffi_symbol kata_rt_string_contains"
    );
}

// ── Tipos intrínsecos: resolve_type_expr mapeia corretamente ─────

#[test]
fn signatures_dos_metodos_estao_no_resolved_module() {
    let resolved = load_stdlib_for_tests().expect("prelude deve resolver");

    // As signatures dos métodos de implements devem estar em resolved.signatures
    // para que o DispatchTable as receba.
    // Procura por "next" com param Array::(A)
    let next_sigs: Vec<_> = resolved
        .signatures
        .iter()
        .filter(|s| s.name == "next")
        .collect();
    assert!(
        !next_sigs.is_empty(),
        "Deve haver signatures para 'next' (ITERABLE)"
    );

    // Procura por "len" (COUNTABLE)
    let len_sigs: Vec<_> = resolved
        .signatures
        .iter()
        .filter(|s| s.name == "len")
        .collect();
    assert!(
        !len_sigs.is_empty(),
        "Deve haver signatures para 'len' (COUNTABLE)"
    );

    // Procura por "at" (INDEXABLE)
    let at_sigs: Vec<_> = resolved
        .signatures
        .iter()
        .filter(|s| s.name == "at")
        .collect();
    assert!(
        !at_sigs.is_empty(),
        "Deve haver signatures para 'at' (INDEXABLE)"
    );

    // Procura por "contains" (CONTAINS)
    let contains_sigs: Vec<_> = resolved
        .signatures
        .iter()
        .filter(|s| s.name == "contains")
        .collect();
    assert!(
        !contains_sigs.is_empty(),
        "Deve haver signatures para 'contains' (CONTAINS)"
    );
}

// ── resolve_type_expr produz Ty::List/Array/Range ────────────────

#[test]
fn resolve_type_expr_mapea_tipos_intrinsecos() {
    use kata_lexer::lex;
    use kata_parser::parse;
    use kata_resolution::resolve;

    // Testa que List::(Int) resolve para Ty::List(Int)
    let src = "x :: List::(Int) => List::(Int)";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let user = resolve(&module).expect("resolve");

    // Procura a signature de x
    let x_sig = user
        .signatures
        .iter()
        .find(|s| s.name == "x")
        .expect("signature x deve existir");

    assert!(
        matches!(&x_sig.param_types[0], Ty::List(inner) if matches!(inner.as_ref(), Ty::Prim(_))),
        "List::(Int) deve resolver para Ty::List(Int), got {:?}",
        x_sig.param_types[0]
    );
    assert!(
        matches!(&x_sig.return_type, Ty::List(inner) if matches!(inner.as_ref(), Ty::Prim(_))),
        "return type deve ser Ty::List(Int), got {:?}",
        x_sig.return_type
    );
}

#[test]
fn resolve_type_expr_mapeia_array_e_range() {
    use kata_lexer::lex;
    use kata_parser::parse;
    use kata_resolution::resolve;

    let src = "f :: Array::(Float) => Range::(Int)";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let user = resolve(&module).expect("resolve");

    let f_sig = user
        .signatures
        .iter()
        .find(|s| s.name == "f")
        .expect("signature f deve existir");

    assert!(
        matches!(&f_sig.param_types[0], Ty::Array(inner) if matches!(inner.as_ref(), Ty::Prim(_))),
        "Array::(Float) deve resolver para Ty::Array(Float), got {:?}",
        f_sig.param_types[0]
    );
    assert!(
        matches!(&f_sig.return_type, Ty::Range(inner) if matches!(inner.as_ref(), Ty::Prim(_))),
        "Range::(Int) deve resolver para Ty::Range(Int), got {:?}",
        f_sig.return_type
    );
}
