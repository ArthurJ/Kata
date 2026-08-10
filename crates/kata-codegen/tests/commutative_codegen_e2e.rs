//! Testes E2E de codegen de `@commutative` cross-type.
//!
//! Prova que o fix do swap comutativo chega ao codegen: quando `@commutative`
//! inverte os tipos dos args para casar com a overload, os `typed_args` na
//! TAST também são reordenados para que o codegen receba os args na ordem
//! esperada pela overload.
//!
//! Sem o fix: o swap acontece só nos tipos (resolve_inner), mas os args
//! na TAST permanecem na ordem original. O codegen trata arg[0] (bits de
//! Float) como Int e arg[1] (bits de Int) como Float → corrompido.
//!
//! Com o fix: `DispatchOutcome { swapped: true }` faz `apply_dispatch.rs`
//! reordenar `typed_args` antes de construir o `Closure`.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

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

fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex");
    let module = parse(tokens).expect("parse");
    let prelude = load_prelude().expect("prelude");
    let user = resolve(&module).expect("resolve");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr()).expect("codegen+JIT");
    (jit.raw, jit.ty)
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ── Prova que o swap chega ao codegen ──────────────────────────

/// `@commutative` em função cross-type Int→Float.
/// Declara `first :: Int Float => Float @commutative` com lambda que retorna f.
/// Chamada `first 2.5 10` (Float Int) → swap → (Int Float) → codegen
/// recebe args reordenados [10, 2.5] → lambda i=10 f=2.5 → retorna 2.5.
///
/// Sem o fix: args chegariam [2.5, 10] → i=2.5(bits de f64 como Int SMI)
/// f=10(bits de Int SMI como f64) → retorna lixo (não 2.5).
#[test]
fn commutative_cross_type_swap_codegen_float_result() {
    let src = "\
@commutative
first :: Int Float => Float
lambda i f: f
first 2.5 10
";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::float(), "tipo de retorno deve ser Float");
    let f = f64::from_bits(raw as u64);
    assert!(
        (f - 2.5).abs() < 0.001,
        "esperado 2.5, got {f} (se for lixo, o swap não chegou ao codegen)"
    );
}

/// Mesma função, mas retorna i (Int). O swap reordena args para que
/// o Int chegue na posição correta.
/// `first_int :: Int Float => Int @commutative` com lambda que retorna i.
/// Chamada `first_int 2.5 10` (Float Int) → swap → (Int Float) → retorna 10.
#[test]
fn commutative_cross_type_swap_codegen_int_result() {
    let src = "\
@commutative
first_int :: Int Float => Int
lambda i f: i
first_int 2.5 10
";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int(), "tipo de retorno deve ser Int");
    assert_eq!(
        untag_smi(raw),
        10,
        "esperado 10 (se for lixo, o swap não chegou ao codegen)"
    );
}

/// Match direto (sem swap) — controle. Args já compatíveis.
/// `first 10 2.5` (Int Float) → match direto → sem swap → retorna 2.5.
#[test]
fn commutative_cross_type_no_swap_direct_match() {
    let src = "\
@commutative
first :: Int Float => Float
lambda i f: f
first 10 2.5
";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::float());
    let f = f64::from_bits(raw as u64);
    assert!(
        (f - 2.5).abs() < 0.001,
        "esperado 2.5 (match direto, sem swap), got {f}"
    );
}

/// `@commutative` com dois args Float (same-type) — swap é no-op na prática.
/// `+ 3.14 2.71` (Float Float) → match direto → sem swap.
#[test]
fn commutative_same_type_float_add_no_swap() {
    let src = "+ 3.14 2.71";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::float());
    let f = f64::from_bits(raw as u64);
    assert!(
        (f - 5.85).abs() < 0.01,
        "esperado ~5.85, got {f}"
    );
}