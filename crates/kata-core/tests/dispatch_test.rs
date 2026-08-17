use kata_core::interface_registry::InterfaceRegistry;
use kata_core::{DispatchError, DispatchTable, OverloadInfo, Ty};

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
        param_defaults: vec![],
    }
}

// ── Resolução básica: 1 overload ──────────────────────────────

#[test]
fn resolve_single_overload_exact_match() {
    let mut table = DispatchTable::new();
    table.insert(make_ffi_info(
        "+",
        &[Ty::int(), Ty::int()],
        Ty::int(),
        "kata_rt_bi_add",
    ));

    let result = table.resolve("+", &[Ty::int(), Ty::int()], &InterfaceRegistry::new());
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().ffi_symbol.as_deref(),
        Some("kata_rt_bi_add")
    );
}

#[test]
fn resolve_single_overload_type_mismatch() {
    let mut table = DispatchTable::new();
    table.insert(make_ffi_info(
        "+",
        &[Ty::int(), Ty::int()],
        Ty::int(),
        "kata_rt_bi_add",
    ));

    let result = table.resolve("+", &[Ty::int(), Ty::float()], &InterfaceRegistry::new());
    assert!(result.is_err());
}

#[test]
fn resolve_function_not_found() {
    let table = DispatchTable::new();
    let result = table.resolve("nonexistent", &[Ty::int()], &InterfaceRegistry::new());
    assert_eq!(
        result.unwrap_err(),
        DispatchError::FunctionNotFound {
            name: "nonexistent".to_string(),
            arg_count: 1
        }
    );
}

#[test]
fn resolve_arity_mismatch() {
    let mut table = DispatchTable::new();
    table.insert(make_ffi_info(
        "+",
        &[Ty::int(), Ty::int()],
        Ty::int(),
        "kata_rt_bi_add",
    ));

    // Pass 3 args para função de 2 params
    let result = table.resolve(
        "+",
        &[Ty::int(), Ty::int(), Ty::int()],
        &InterfaceRegistry::new(),
    );
    assert!(result.is_err());
}

// ── Múltiplas overloads: o coração do scoring ─────────────────

#[test]
fn resolve_multiple_overloads_selects_exact() {
    let mut table = DispatchTable::new();
    // + :: Int Int => Int
    table.insert(make_ffi_info(
        "+",
        &[Ty::int(), Ty::int()],
        Ty::int(),
        "kata_rt_bi_add",
    ));
    // + :: Float Float => Float
    table.insert(make_ffi_info(
        "+",
        &[Ty::float(), Ty::float()],
        Ty::float(),
        "kata_rt_fadd",
    ));
    // + :: Rational Rational => Rational
    table.insert(make_ffi_info(
        "+",
        &[Ty::rational(), Ty::rational()],
        Ty::rational(),
        "kata_rt_rat_add",
    ));

    // Args Int Int → deve selecionar a overload Int
    let result = table.resolve("+", &[Ty::int(), Ty::int()], &InterfaceRegistry::new());
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().ffi_symbol.as_deref(),
        Some("kata_rt_bi_add")
    );
}

#[test]
fn resolve_multiple_overloads_selects_float() {
    let mut table = DispatchTable::new();
    table.insert(make_ffi_info(
        "+",
        &[Ty::int(), Ty::int()],
        Ty::int(),
        "kata_rt_bi_add",
    ));
    table.insert(make_ffi_info(
        "+",
        &[Ty::float(), Ty::float()],
        Ty::float(),
        "kata_rt_fadd",
    ));
    table.insert(make_ffi_info(
        "+",
        &[Ty::rational(), Ty::rational()],
        Ty::rational(),
        "kata_rt_rat_add",
    ));

    // Args Float Float → deve selecionar a overload Float
    let result = table.resolve("+", &[Ty::float(), Ty::float()], &InterfaceRegistry::new());
    assert!(result.is_ok());
    assert_eq!(result.unwrap().ffi_symbol.as_deref(), Some("kata_rt_fadd"));
}

#[test]
fn resolve_multiple_overloads_selects_rational() {
    let mut table = DispatchTable::new();
    table.insert(make_ffi_info(
        "+",
        &[Ty::int(), Ty::int()],
        Ty::int(),
        "kata_rt_bi_add",
    ));
    table.insert(make_ffi_info(
        "+",
        &[Ty::float(), Ty::float()],
        Ty::float(),
        "kata_rt_fadd",
    ));
    table.insert(make_ffi_info(
        "+",
        &[Ty::rational(), Ty::rational()],
        Ty::rational(),
        "kata_rt_rat_add",
    ));

    let result = table.resolve(
        "+",
        &[Ty::rational(), Ty::rational()],
        &InterfaceRegistry::new(),
    );
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().ffi_symbol.as_deref(),
        Some("kata_rt_rat_add")
    );
}

#[test]
fn resolve_mixed_args_no_match() {
    let mut table = DispatchTable::new();
    table.insert(make_ffi_info(
        "+",
        &[Ty::int(), Ty::int()],
        Ty::int(),
        "kata_rt_bi_add",
    ));
    table.insert(make_ffi_info(
        "+",
        &[Ty::float(), Ty::float()],
        Ty::float(),
        "kata_rt_fadd",
    ));

    // Int + Float → nenhuma overload compatível
    let result = table.resolve("+", &[Ty::int(), Ty::float()], &InterfaceRegistry::new());
    assert!(result.is_err());
}

// ── Commutative ───────────────────────────────────────────────

#[test]
fn resolve_commutative_swaps_args() {
    let mut table = DispatchTable::new();
    // Só existe overload Int Float (não Float Int)
    table.insert(make_ffi_info(
        "==",
        &[Ty::int(), Ty::float()],
        Ty::boolean(),
        "kata_rt_eq",
    ));
    table.mark_commutative("==");

    // Float Int → commutative swap → Int Float → match
    let result = table.resolve("==", &[Ty::float(), Ty::int()], &InterfaceRegistry::new());
    assert!(result.is_ok());
    assert_eq!(result.unwrap().ffi_symbol.as_deref(), Some("kata_rt_eq"));
}

#[test]
fn resolve_commutative_no_swap_if_direct_match() {
    let mut table = DispatchTable::new();
    table.insert(make_ffi_info(
        "==",
        &[Ty::int(), Ty::int()],
        Ty::boolean(),
        "kata_rt_eq",
    ));
    table.mark_commutative("==");

    // Int Int → match direto, sem swap
    let result = table.resolve("==", &[Ty::int(), Ty::int()], &InterfaceRegistry::new());
    assert!(result.is_ok());
}

// ── Score 4D ──────────────────────────────────────────────────

#[test]
fn score_exact_beats_incompatible() {
    use kata_core::Score;

    let compatible = Score {
        exact: 2,
        alias: 0,
        refined: 0,
        iface: 0,
        is_generic_origin: false,
    };
    let incompatible = Score::incompatible();

    assert!(compatible > incompatible);
    assert!(!incompatible.is_compatible(2));
    assert!(compatible.is_compatible(2));
}

#[test]
fn score_ordering_exact_vs_alias() {
    use kata_core::Score;

    let more_exact = Score {
        exact: 2,
        alias: 0,
        refined: 0,
        iface: 0,
        is_generic_origin: false,
    };
    let more_alias = Score {
        exact: 1,
        alias: 1,
        refined: 0,
        iface: 0,
        is_generic_origin: false,
    };

    // Mais exact vence (lexicográfico)
    assert!(more_exact > more_alias);
}

#[test]
fn score_ordering_concrete_beats_generic() {
    use kata_core::Score;

    let concrete = Score {
        exact: 2,
        alias: 0,
        refined: 0,
        iface: 0,
        is_generic_origin: false,
    };
    let generic = Score {
        exact: 2,
        alias: 0,
        refined: 0,
        iface: 0,
        is_generic_origin: true,
    };

    // Concreto (false) vence genérico (true)
    assert!(
        concrete < generic,
        "false < true no Ord, mas concreto vence"
    );
    // Na ordenação decrescente do dispatch, concreto deve vir primeiro
    assert!(concrete.cmp(&generic) == std::cmp::Ordering::Less);
}
