//! Testes E2E de pattern matching de variantes não-qualificadas em match arms.
//!
//! Valida que `Ok v:`, `Err _:`, `Some n:`, `None:` funcionam sem qualificação
//! (`Result::Ok v`, `Optional::Some n`). O parser produz `Pattern::Variant`
//! com `enum_name` vazio; o typeck resolve via `EnumRegistry` do scrutinee.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::jit_eval;
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
    let jit = jit_eval(&typed, &Default::default()).expect("codegen+JIT deve succeed");
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
    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);
    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry: {
            let mut ir = prelude.interface_registry.clone();
            ir.merge(user.interface_registry.clone());
            ir
        },
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
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
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// `Ok v:` em match sobre Result — desempacota o valor.
#[test]
fn unqualified_ok_pattern_desempacota_result() {
    let src = r#"action extrai => Int
    let r := Result::Ok 42
    match r
        Ok v: v
        Err _: 0
        otherwise: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// `Err _:` em match sobre Result — cai no braço de erro.
#[test]
fn unqualified_err_pattern_cai_no_erro() {
    let src = r#"action extrai => Int
    let r := Result::Err 99
    match r
        Ok v: v
        Err _: 0
        otherwise: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}

/// `Some n:` em match sobre Optional — desempacota o valor.
#[test]
fn unqualified_some_pattern_desempacota_optional() {
    let src = r#"action extrai => Int
    let a := Optional::Some 7
    match a
        Some n: n
        None: 0
        otherwise: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 7);
}

/// `None:` em match sobre Optional — cai no braço vazio.
#[test]
fn unqualified_none_pattern_cai_no_vazio() {
    let src = r#"action extrai => Int
    let a := Optional::None
    match a
        Some n: n
        None: 0
        otherwise: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}

/// Match sobre refined constructor com `Ok v:` / `Err _:` sem qualificação.
/// PositiveInt 42 retorna Result::(PositiveInt, Text).
/// O `v` em `Ok v` é PositiveInt — downcast via `::Int` para retornar Int.
#[test]
fn unqualified_pattern_com_refined_constructor() {
    let src = r#"data (Int, > _ 0) as PositiveInt
action extrai => Int
    let r := PositiveInt 42
    match r
        Ok v: v::Int
        Err _: 0
        otherwise: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// Match sobre refined constructor que falha — `Err _:` captura o erro.
#[test]
fn unqualified_pattern_refined_constructor_falha() {
    let src = r#"data (Int, > _ 0) as PositiveInt
action extrai => Int
    let r := PositiveInt -5
    match r
        Ok v: v::Int
        Err _: 0
        otherwise: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}
