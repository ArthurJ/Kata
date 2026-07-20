//! Testes E2E de codegen de Sum com payload.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Valida DoD 14-18: construção de Sum com payload, match com extração de payload,
//! match em 3+ variantes, Sum como ponteiro, e funções FFI do runtime.

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Combina prelude + módulo do usuário (replica do driver).
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    // Merge enum_registry: prelude + user (user enums sobrescrevem prelude).
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: {
            let mut ir = prelude.interface_registry.clone();
            ir.merge(user.interface_registry.clone());
            ir
        },
        functions: {
            let mut fns = prelude.functions;
            let user_fn_names: std::collections::HashSet<&str> =
                user.functions.iter().map(|f| f.name.as_str()).collect();
            fns.retain(|f| !user_fn_names.contains(f.name.as_str()));
            fns.extend(user.functions);
            fns
        },
        actions: {
            let mut acts = prelude.actions;
            let user_action_names: std::collections::HashSet<&str> =
                user.actions.iter().map(|a| a.name.as_str()).collect();
            acts.retain(|a| !user_action_names.contains(a.name.as_str()));
            acts.extend(user.actions);
            acts
        },
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// DoD 14: `Result::Ok 42` constrói Sum com payload. Match extrai payload.
#[test]
fn sum_constrói_e_extrai_payload() {
    let src = r#"enum Result
    Ok(Int)
    Err(Int)

match Result::Ok 42
    Result::Ok v: v
    Result::Err e: e"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        42,
        "match Result::Ok 42 deve extrair payload 42"
    );
}

/// DoD 14 (complementar): `Result::Err 99` constrói Sum com payload e match extrai.
#[test]
fn sum_err_constrói_e_extrai_payload() {
    let src = r#"enum Result
    Ok(Int)
    Err(Int)

match Result::Err 99
    Result::Ok v: v
    Result::Err e: e"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        99,
        "match Result::Err 99 deve extrair payload 99"
    );
}

/// DoD 15: `Optional::Some 42` e `Optional::None` funcionam.
#[test]
fn optional_some_extrai_payload() {
    let src = r#"enum Optional
    Some(Int)
    None

match Optional::Some 42
    Optional::Some v: v
    Optional::None: 0"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        42,
        "match Optional::Some 42 deve extrair 42"
    );
}

/// DoD 15: `Optional::None` — variante unitária no mesmo enum que variante com payload.
#[test]
fn optional_none_match_retorna_default() {
    let src = r#"enum Optional
    Some(Int)
    None

match Optional::None
    Optional::Some v: v
    Optional::None: 0"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        0,
        "match Optional::None deve cair no braço None"
    );
}

/// DoD 16: Match em 3+ variantes (general case) executa sem trap.
#[test]
fn match_tres_variantes_general_case() {
    let src = r#"enum Traffic
    Red(Int)
    Yellow(Int)
    Green(Int)

match Traffic::Red 100
    Traffic::Red v: v
    Traffic::Yellow v: v
    Traffic::Green v: v"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        100,
        "match Traffic::Red 100 deve extrair payload 100"
    );
}

/// DoD 16 (complementar): Match em 3+ variantes — segunda variante.
#[test]
fn match_tres_variantes_segunda() {
    let src = r#"enum Traffic
    Red(Int)
    Yellow(Int)
    Green(Int)

match Traffic::Yellow 50
    Traffic::Red v: v
    Traffic::Yellow v: v
    Traffic::Green v: v"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        50,
        "match Traffic::Yellow 50 deve extrair payload 50"
    );
}

/// DoD 16 (complementar): Match em 3+ variantes — terceira variante.
#[test]
fn match_tres_variantes_terceira() {
    let src = r#"enum Traffic
    Red(Int)
    Yellow(Int)
    Green(Int)

match Traffic::Green 7
    Traffic::Red v: v
    Traffic::Yellow v: v
    Traffic::Green v: v"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        7,
        "match Traffic::Green 7 deve extrair payload 7"
    );
}

/// DoD 17+18: Sum com payload é sempre ponteiro (box). As funções FFI
/// `kata_rt_store_sum_result` e `kata_rt_sum_tag_int` são exercitadas
/// indiretamente por todos os testes acima. Este teste valida que a
/// construção + extração funciona dentro de uma Action, não apenas
/// como expressão top-level.
#[test]
fn sum_dentro_de_action() {
    let src = r#"enum Result
    Ok(Int)
    Err(Int)

action extrai_ok => Int
    match Result::Ok 42
        Result::Ok v: v
        Result::Err e: e
extrai_ok!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        42,
        "Action que extrai payload de Result::Ok 42 deve retornar 42"
    );
}
