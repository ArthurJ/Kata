//! Testes E2E de codegen de Generics de Enum (Result/Optional no prelude).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Valida DoD 19: Result::(T, E) com type params posicionais resolve no typeck.

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_tree_shaking::tree_shake;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

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
        interface_registry: { let mut ir = prelude.interface_registry.clone(); ir.merge(user.interface_registry.clone()); ir },
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

/// DoD 19: `Result::Ok 42` com Result do prelude (não definido pelo usuário).
/// O typeck infere T=Int do argumento e produz Ty::Generic("Result", [Int, Var(E)]).
#[test]
fn result_ok_do_prelude() {
    let src = r#"match Result::Ok 42
    Result::Ok v: v
    Result::Err e: e"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// DoD 19: `Result::Err 99` com Result do prelude.
#[test]
fn result_err_do_prelude() {
    let src = r#"match Result::Err 99
    Result::Ok v: v
    Result::Err e: e"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 99);
}

/// DoD 19: `Optional::Some 42` com Optional do prelude.
#[test]
fn optional_some_do_prelude() {
    let src = r#"match Optional::Some 42
    Optional::Some v: v
    Optional::None: 0"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// DoD 19: `Optional::None` — variante unitária de enum genérico do prelude.
#[test]
fn optional_none_do_prelude() {
    let src = r#"match Optional::None
    Optional::Some v: v
    Optional::None: 0"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}

/// DoD 19: Result com tipos diferentes — Ok(Int) e Err(Text).
/// Valida que T e E são inferidos independentemente.
#[test]
fn result_com_tipos_diferentes() {
    let src = r#"match Result::Ok 42
    Result::Ok v: v
    Result::Err e: 0"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// DoD 19: Match em Result dentro de uma Action.
#[test]
fn result_dentro_de_action() {
    let src = r#"action extrai_ok => Int
    match Result::Ok 42
        Result::Ok v: v
        Result::Err e: e
extrai_ok!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// DoD 19: Match em Result dentro de uma Action.
#[test]
fn optional_none_dentro_de_action() {
    let src = r#"action extrai_optional => Int
    match Optional::None
        Optional::Some v: v
        Optional::None: 0
extrai_optional!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}

/// DoD 19: Enums do usuário ainda funcionam (não-genéricos) ao lado de generics do prelude.
#[test]
fn enum_usuario_nao_generico_funciona() {
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
    assert_eq!(untag_smi(raw), 100);
}
