//! Testes E2E de codegen de coerção contextual no `|` e grouped ascription.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//!
//! DoDs cobertos:
//! - DoD 5: Coerção contextual no `|` com refined types
//! - DoD 12: Grouped ascription `((expr))::Type` — barreira de hint

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::StructKey;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, Signature, load_prelude, resolve};
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

fn merge_resolved_with_extra_sigs(
    prelude: ResolvedModule,
    user: ResolvedModule,
    extra: Vec<Signature>,
) -> ResolvedModule {
    let mut resolved = merge_resolved(prelude, user);
    resolved.signatures.extend(extra);
    resolved
}

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
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

fn eval_src_with_extra(src: &str, extra: Vec<Signature>) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved_with_extra_sigs(prelude, user, extra);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

fn infer_fails(src: &str) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let module = match parse(tokens) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let prelude = match load_prelude() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let user = match resolve(&module) {
        Ok(u) => u,
        Err(_) => return true,
    };
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).is_err()
}

fn infer_fails_with_extra(src: &str, extra: Vec<Signature>) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let module = match parse(tokens) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let prelude = match load_prelude() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let user = match resolve(&module) {
        Ok(u) => u,
        Err(_) => return true,
    };
    let resolved = merge_resolved_with_extra_sigs(prelude, user, extra);
    infer_module(&module, &resolved).is_err()
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ── DoD 5: Coerção contextual no `|` com refined types ──────────────

/// DoD 5: `Some(5::PositiveInt) | 1` desempacota 5.
/// O fallback `1` é implicitamente tratado como `PositiveInt` (predicado
/// `> _ 0` validado em compile-time). O resultado é o payload desempacotado.
#[test]
fn dod5_pipe_fallback_coercao_refined_valida() {
    let src = "data (Int, > _ 0) as PositiveInt\nSome(5::PositiveInt) | 1";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct(StructKey::Plain("PositiveInt".into())));
    assert_eq!(untag_smi(raw), 5);
}

/// DoD 5 (erro): `Some(5::PositiveInt) | 0` é type error.
/// O fallback `0` falha o predicado `> _ 0` — 0 não é positivo.
#[test]
fn dod5_pipe_fallback_coercao_refined_falha() {
    let src = "data (Int, > _ 0) as PositiveInt\nSome(5::PositiveInt) | 0";
    assert!(
        infer_fails(src),
        "Some(5::PositiveInt) | 0 deve falhar (predicado > _ 0 não satisfeito)"
    );
}

/// DoD 5 (fallback sem refined): `Some(42) | 0` funciona normalmente.
/// Sem refined type no payload, o `|` funciona como antes — sem coerção.
#[test]
fn dod5_pipe_fallback_sem_refined_funciona() {
    let src = "Some(42) | 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 42);
}

/// DoD 5 (None): `None | 42` → 42 (fallback).
#[test]
fn dod5_pipe_fallback_none_retorna_fallback() {
    let src = "None | 42";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 42);
}

// ── DoD 12: Grouped ascription `((expr))::Type` ─────────────────────

/// DoD 12: `((/ 1 3))::Int` → OK.
/// Grouped: sem hint, `/ 1 3` despacha para (Int, Int) → Int.
/// Depois valida Int == Int → confirma.
#[test]
fn dod12_grouped_ascription_confirma_int() {
    let src = "((/ 1 3))::Int";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 0);
}

/// DoD 12 (erro): `((/ 1 3))::Rational` → type error.
/// Grouped: sem hint, `/ 1 3` despacha para (Int, Int) → Int.
/// Depois valida Int ≠ Rational → type mismatch.
#[test]
fn dod12_grouped_ascription_rational_falha() {
    let src = "((/ 1 3))::Rational";
    assert!(
        infer_fails(src),
        "((/ 1 3))::Rational deve falhar — grouped barreira impede hint, / despacha Int"
    );
}

/// DoD 12 (com Float): `((/ 1.0 2.0))::Float` → OK.
/// Grouped: sem hint, `/ 1.0 2.0` despacha para (Float, Float) → Float.
#[test]
fn dod12_grouped_ascription_float_confirma() {
    let src = "((/ 1.0 2.0))::Float";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::float());
    let f = f64::from_bits(raw as u64);
    assert!((f - 0.5).abs() < 0.001, "esperado 0.5, got {f}");
}

/// DoD 12 vs grouped ascription: diferença entre `(expr)::Type` e `((expr))::Type`
/// com múltiplas overloads. Com função custom de 2 overloads mesmo arg shape:
/// - `(custom 42)::Int` → hint Int seleciona Int→Int (ret-directed)
/// - `((custom 42))::Int` → sem hint, AmbiguousDispatch (barreira impede hint)
fn custom_overloads() -> Vec<Signature> {
    vec![
        Signature {
            name: "custom".into(),
            param_types: vec![Ty::int()],
            return_type: Ty::int(),
            ffi_symbol: Some("kata_rt_tag_int".into()),
            is_associative: false,
            associative_neutral: None,
            is_action: false,
            is_commutative: false,
            type_params: vec![],
        },
        Signature {
            name: "custom".into(),
            param_types: vec![Ty::int()],
            return_type: Ty::text(),
            ffi_symbol: Some("kata_rt_bi_show".into()),
            is_associative: false,
            associative_neutral: None,
            is_action: false,
            is_commutative: false,
            type_params: vec![],
        },
    ]
}

/// `(custom 42)::Int` → ret-directed: hint Int seleciona overload Int→Int.
#[test]
fn dod12_ret_directed_seleciona_com_hint() {
    let src = "(custom 42)::Int";
    let (raw, ty) = eval_src_with_extra(src, custom_overloads());
    assert_eq!(ty, Ty::int());
    // double-tagging (ver test de grouped ascription), mas prova dispatch Int→Int
    assert_eq!(untag_smi(raw), 85);
}

/// `((custom 42))::Int` → grouped: barreira impede hint.
/// Sem hint, 2 overloads de mesmo score → AmbiguousDispatch.
#[test]
fn dod12_grouped_barreira_causa_ambiguous() {
    let src = "((custom 42))::Int";
    assert!(
        infer_fails_with_extra(src, custom_overloads()),
        "((custom 42))::Int deve falhar — grouped barreira impede hint, AmbiguousDispatch"
    );
}
