//! Prelude signatures — catálogo hardcoded de assinaturas do prelude.
//!
//! `load_prelude` constrói o `ResolvedModule` com os tipos builtin
//! (Int, Float, Text, Rational, Boolean, Unit) e as assinaturas FFI
//! de todos os operadores e funções do prelude.
//!
//! Fio 10 substitui isto por carregamento de `stdlib/core.kata` do filesystem.

use kata_core::{EnumRegistry, PrimTy, Ty, TypeEnv, VariantInfo};

use crate::{ResolvedModule, Signature};

/// Carrega o prelude hardcoded e retorna o TypeEnv + signatures populados.
/// O driver chama isto antes de resolver o módulo do usuário.
pub fn load_prelude() -> Result<ResolvedModule, Vec<crate::ResolveError>> {
    // O prelude é parseado como um módulo Kata normal
    // Por enquanto, construímos o TypeEnv manualmente — Fio 10 fará parse real
    let mut type_env = TypeEnv::new();

    // Tipos do prelude
    type_env.define("Int", Ty::Prim(PrimTy::Int));
    type_env.define("Float", Ty::Prim(PrimTy::Float));
    type_env.define("Text", Ty::Prim(PrimTy::Text));
    type_env.define("Rational", Ty::Prim(PrimTy::Rational));
    type_env.define("Boolean", Ty::Sum("Boolean".into()));
    type_env.define("Unit", Ty::Unit);

    // Variantes de Boolean
    // (serão registradas como construtores no DispatchTable na inferência)
    let mut enum_registry = EnumRegistry::new();
    enum_registry.register(
        "Boolean",
        vec![
            VariantInfo {
                name: "True".into(),
                payload_ty: None,
            },
            VariantInfo {
                name: "False".into(),
                payload_ty: None,
            },
        ],
    );

    // Fase 6: Result e Optional — enums genéricos do prelude.
    // Result tem type_params ["T", "E"] — Ok carrega T, Err carrega E.
    // Optional tem type_params ["T"] — Some carrega T, None é unitária.
    enum_registry.register_generic(
        "Result",
        vec!["T".into(), "E".into()],
        vec![
            VariantInfo {
                name: "Ok".into(),
                payload_ty: Some(Ty::Var("T".into())),
            },
            VariantInfo {
                name: "Err".into(),
                payload_ty: Some(Ty::Var("E".into())),
            },
        ],
    );
    enum_registry.register_generic(
        "Optional",
        vec!["T".into()],
        vec![
            VariantInfo {
                name: "Some".into(),
                payload_ty: Some(Ty::Var("T".into())),
            },
            VariantInfo {
                name: "None".into(),
                payload_ty: None,
            },
        ],
    );

    // TypeEnv: registra Result e Optional como tipos.
    // No TypeEnv, o tipo base é Ty::Sum — a instanciação (Ty::Generic) acontece
    // no resolve_type_expr quando o usuário escreve Result::(Int, Text).
    type_env.define("Result", Ty::Sum("Result".into()));
    type_env.define("Optional", Ty::Sum("Optional".into()));

    // Assinaturas do prelude
    let signatures = vec![
        // Int aritmética
        sig(
            "+",
            vec![Ty::int(), Ty::int()],
            Ty::int(),
            "kata_rt_bi_add",
            true,
            Some(0),
        ),
        sig(
            "-",
            vec![Ty::int(), Ty::int()],
            Ty::int(),
            "kata_rt_bi_sub",
            false,
            None,
        ),
        sig(
            "*",
            vec![Ty::int(), Ty::int()],
            Ty::int(),
            "kata_rt_bi_mul",
            true,
            Some(1),
        ),
        sig(
            "/",
            vec![Ty::int(), Ty::int()],
            Ty::int(),
            "kata_rt_bi_div",
            false,
            None,
        ),
        sig(
            "=",
            vec![Ty::int(), Ty::int()],
            Ty::boolean(),
            "kata_rt_bi_eq",
            false,
            None,
        ),
        sig(
            "<",
            vec![Ty::int(), Ty::int()],
            Ty::boolean(),
            "kata_rt_bi_lt",
            false,
            None,
        ),
        sig(
            ">",
            vec![Ty::int(), Ty::int()],
            Ty::boolean(),
            "kata_rt_bi_gt",
            false,
            None,
        ),
        // Float aritmética
        sig(
            "+",
            vec![Ty::float(), Ty::float()],
            Ty::float(),
            "kata_rt_fadd",
            false,
            None,
        ),
        sig(
            "-",
            vec![Ty::float(), Ty::float()],
            Ty::float(),
            "kata_rt_fsub",
            false,
            None,
        ),
        sig(
            "*",
            vec![Ty::float(), Ty::float()],
            Ty::float(),
            "kata_rt_fmul",
            false,
            None,
        ),
        sig(
            "/",
            vec![Ty::float(), Ty::float()],
            Ty::float(),
            "kata_rt_fdiv",
            false,
            None,
        ),
        sig(
            "=",
            vec![Ty::float(), Ty::float()],
            Ty::boolean(),
            "kata_rt_fcmp_eq",
            false,
            None,
        ),
        sig(
            "<",
            vec![Ty::float(), Ty::float()],
            Ty::boolean(),
            "kata_rt_fcmp_lt",
            false,
            None,
        ),
        sig(
            ">",
            vec![Ty::float(), Ty::float()],
            Ty::boolean(),
            "kata_rt_fcmp_gt",
            false,
            None,
        ),
        // Rational aritmética
        sig(
            "+",
            vec![Ty::rational(), Ty::rational()],
            Ty::rational(),
            "kata_rt_rat_add",
            true,
            Some(0),
        ),
        sig(
            "-",
            vec![Ty::rational(), Ty::rational()],
            Ty::rational(),
            "kata_rt_rat_sub",
            false,
            None,
        ),
        sig(
            "*",
            vec![Ty::rational(), Ty::rational()],
            Ty::rational(),
            "kata_rt_rat_mul",
            true,
            Some(1),
        ),
        sig(
            "/",
            vec![Ty::rational(), Ty::rational()],
            Ty::rational(),
            "kata_rt_rat_div",
            false,
            None,
        ),
        sig(
            "=",
            vec![Ty::rational(), Ty::rational()],
            Ty::boolean(),
            "kata_rt_rat_eq",
            false,
            None,
        ),
        sig(
            "<",
            vec![Ty::rational(), Ty::rational()],
            Ty::boolean(),
            "kata_rt_rat_lt",
            false,
            None,
        ),
        sig(
            ">",
            vec![Ty::rational(), Ty::rational()],
            Ty::boolean(),
            "kata_rt_rat_gt",
            false,
            None,
        ),
        // Conversões
        sig(
            "to_float",
            vec![Ty::rational()],
            Ty::float(),
            "kata_rt_rat_to_float",
            false,
            None,
        ),
        sig(
            "from_float",
            vec![Ty::float()],
            Ty::rational(),
            "kata_rt_rat_from_float",
            false,
            None,
        ),
        sig(
            "from_int",
            vec![Ty::int()],
            Ty::rational(),
            "kata_rt_int_to_rational",
            false,
            None,
        ),
        // I/O — echo é uma Action builtin (Fio 3)
        sig_action("echo", vec![Ty::text()], Ty::Unit, "kata_rt_print"),
        // Show
        sig(
            "show",
            vec![Ty::int()],
            Ty::text(),
            "kata_rt_bi_show",
            false,
            None,
        ),
        sig(
            "show",
            vec![Ty::rational()],
            Ty::text(),
            "kata_rt_rat_show",
            false,
            None,
        ),
    ];

    Ok(ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        functions: Vec::new(),
        actions: Vec::new(),
    })
}

/// Helper para construir Signature
fn sig(
    name: &str,
    params: Vec<Ty>,
    ret: Ty,
    ffi: &str,
    assoc: bool,
    neutral: Option<i64>,
) -> Signature {
    Signature {
        name: name.to_string(),
        param_types: params,
        return_type: ret,
        ffi_symbol: Some(ffi.to_string()),
        is_associative: assoc,
        associative_neutral: neutral,
        is_action: false,
    }
}

/// Helper para construir Signature de Action builtin (Fio 3).
fn sig_action(name: &str, params: Vec<Ty>, ret: Ty, ffi: &str) -> Signature {
    Signature {
        name: name.to_string(),
        param_types: params,
        return_type: ret,
        ffi_symbol: Some(ffi.to_string()),
        is_associative: false,
        associative_neutral: None,
        is_action: true,
    }
}
