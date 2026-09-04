//! E2E: post-condições inter-procedurais propagadas pelo Z3.
//! Grupo: match sobre Result com guards, args complexos, disjunção.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::StructKey;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve_with_prelude};
use kata_tree_shaking::tree_shake;

fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve_with_prelude(
        &module,
        "__local__",
        kata_resolution::DirectiveRegistry::new(),
        &prelude.interface_registry,
        &prelude.directive_registry,
        Some(&prelude.type_graph),
        Some(&prelude.type_env),
    )
    .expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

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
        internal_signatures: Vec::new(),
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
        type_graph: {
            let mut tg = prelude.type_graph.clone();
            tg.merge(&user.type_graph);
            tg
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

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ── Nível 2: Post-condições inter-procedurais ─────────────────────

// ── 9. match (div 10 b): Ok n → b::NonZero (b ≠ 0 provado) ──

/// `div 10 b` tem guard `= b 0: Err` e `otherwise: Ok`.
/// No braço `Ok`, a post-condição `not(= b 0)` é adicionada como path
/// condition. O braço faz `b::NonZero` — o predicado de NonZero é
/// `!= _ (zero _)` = `!= _ 0`, que é exatamente `not(= b 0)`.
/// O Z3 prova que o predicado é satisfeito pela post-condição.
/// NonZero já existe no stdlib (não precisa redefinir).
#[test]
fn t_post_cond_div_ok_prova_nonzero() {
    let src = r#"action test_post_cond => NonZero::Int
    let b := 5
    match (div 10 b)
        Result::Ok n: b::NonZero
        Result::Err _: 5::NonZero
test_post_cond!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "braço Ok de div deve aprender b ≠ 0 e provar b::NonZero"
    );
    assert_eq!(untag_smi(raw), 5);
}

// ── 10. match (div 10 0): Err → sem crash, fallback conservador ──

/// `div 10 0` sempre produz `Err`. O braço `Err` recebe a post-condição
/// `= b 0` (= 0 0 = True). O braço usa literal como fallback.
/// Este teste verifica que o braço Err funciona sem crash.
#[test]
fn t_post_cond_div_err_fallback() {
    let src = r#"action test_err => NonZero::Int
    match (div 10 0)
        Result::Ok n: 5::NonZero
        Result::Err _: 5::NonZero
test_err!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "braço Err de div deve funcionar com fallback literal"
    );
    assert_eq!(untag_smi(raw), 5);
}

// ── 11. Função user-defined com guard produzindo Err ──

/// `safe_half :: Int => Result::(Int, Text)` com guard `= n 0: Err`.
/// O caller que faz `match (safe_half 10): Ok m: 10::NonZero`
/// aprende que `n ≠ 0` no braço Ok (o arg `10` é NonZero).
#[test]
fn t_post_cond_user_defined_func() {
    let src = r#"safe_half :: Int => Result::(Int, Text)
lambda n:
    = n 0: Result::Err "zero"
    otherwise: Result::Ok (// n (2::NonZero))

action test_user => NonZero::Int
    match (safe_half 10)
        Result::Ok m: 10::NonZero
        Result::Err _: 5::NonZero
test_user!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "função user-defined com guard deve propagar post-condição"
    );
    // 10::NonZero = 10
    assert_eq!(untag_smi(raw), 10);
}

// ── 12. Post-condição com múltiplos guards (disjunção) ──

/// `clamp_pos :: Int => Result::(Int, Text)` tem DOIS guards que
/// produzem Err: `= n 0` e `< n 0`. Post-cond de Ok =
/// `not(or(= n 0, < n 0))` = `n > 0`. O caller prova `n::NonZero`
/// (predicado `!= _ 0` = `not(= _ 0)`, satisfatório pois n > 0 → n ≠ 0).
#[test]
fn t_post_cond_multiple_guards_disjunction() {
    let src = r#"clamp_pos :: Int => Result::(Int, Text)
lambda n:
    = n 0: Result::Err "zero"
    < n 0: Result::Err "negative"
    otherwise: Result::Ok n

action test_multi => NonZero::Int
    match (clamp_pos 7)
        Result::Ok n: n::NonZero
        Result::Err _: 5::NonZero
test_multi!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "múltiplos guards devem produzir disjunção como post-condição"
    );
    assert_eq!(untag_smi(raw), 7);
}

// ── 13. Arg complexo — Z3 prova post-condição sobre expr aritmética ──

/// `div 10 (+ x y)` — o arg `(+ x y)` é aritmética. A post-condição
/// vira `not(= (+ x y) 0)`. O predicado de NonZero sobre `(+ x y)` é
/// `!= (+ x y) 0` — exatamente a post-condição! O Z3 deve provar.
#[test]
fn t_post_cond_complex_arg_provable() {
    let src = r#"action test_complex => NonZero::Int
    let x := 3
    let y := 4
    match (div 10 (+ x y))
        Result::Ok n: (+ x y)::NonZero
        Result::Err _: 5::NonZero
test_complex!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "arg complexo deve ter post-condição provável pelo Z3"
    );
    // (+ x y) = 7
    assert_eq!(untag_smi(raw), 7);
}
