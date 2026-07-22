//! Testes E2E de codegen de ret-directed dispatch.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//!
//! DoDs cobertos:
//! - DoD 6: `(/ 1 3)::Int` → OK (hint Int compatível com única overload Int→Int)
//! - DoD 7: `(/ 1 3)::Rational` → type error (hint Rational incompatível)
//! - DoD 8: `(/ 1 3)` sem hint → despacha normalmente (única overload)
//! - DoD 8b: função custom com 2 overloads mesmo arg shape, retorno diferente:
//!   - Sem hint → AmbiguousDispatch
//!   - Com hint Int → seleciona Int→Int
//!   - Com hint Text → seleciona Int→Text

use kata_codegen::jit_eval;
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
    }
}

/// merge_resolved + injeção de assinaturas extra no DispatchTable.
/// Usado para o DoD 8b: criar overloads artificiais que o parser não suporta.
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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
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

// ── DoD 6: `(/ 1 3)::Int` → OK ──────────────────────────────────────

/// DoD 6: hint `Int` é compatível com a única overload de `/` (Int, Int) → Int.
/// Resultado: 1/3 truncado = 0.
#[test]
fn dod6_hint_int_seleciona_unica_overload() {
    let src = "(/ 1 3)::Int";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 0);
}

/// DoD 6 com Float: `(/ 1.0 2.0)::Float` → hint Float compatível com
/// a overload (Float, Float) → Float. Resultado: 0.5.
#[test]
fn dod6_hint_float_seleciona_overload_float() {
    let src = "(/ 1.0 2.0)::Float";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::float());
    let f = f64::from_bits(raw as u64);
    assert!((f - 0.5).abs() < 0.001, "esperado 0.5, got {f}");
}

/// Hint `Int` com args Float: `(/ 1.0 2.0)::Int` deve falhar.
/// O hint filtra para (Int, Int) → Int, mas match_score([Float, Float], [Int, Int])
/// é incompatível → TypeMismatch.
#[test]
fn dod6_hint_int_com_args_float_falha() {
    let src = "(/ 1.0 2.0)::Int";
    assert!(
        infer_fails(src),
        "(/ 1.0 2.0)::Int deve falhar — args Float não casam com overload Int"
    );
}

// ── DoD 7: `(/ 1 3)::Rational` → type error ─────────────────────────

/// DoD 7: hint `Rational` não é compatível com nenhuma overload de `/`
/// que aceita args Int. A overload (Int, Int) → Int tem retorno Int ≠ Rational.
/// A overload (Rational, Rational) → Rational tem retorno compatível, mas
/// args Int não casam com params Rational → top_count == 0 → TypeMismatch.
#[test]
fn dod7_hint_rational_incompativel_type_error() {
    let src = "(/ 1 3)::Rational";
    assert!(
        infer_fails(src),
        "(/ 1 3)::Rational deve falhar — / não tem overload Int→Rational"
    );
}

// ── DoD 8: `(/ 1 3)` sem hint → despacha normalmente ────────────────

/// DoD 8: sem hint, `/` com args Int despacha para (Int, Int) → Int.
/// Resultado: 1/3 truncado = 0.
#[test]
fn dod8_sem_hint_despacha_normalmente() {
    let src = "/ 1 3";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 0);
}

// ── DoD 8b: múltiplas overloads, mesmo arg shape, retorno diferente ──

/// Overloads artificiais para o DoD 8b:
/// `custom :: Int => Int` (ffi: kata_rt_tag_int — tag SMI)
/// `custom :: Int => Text` (ffi: kata_rt_bi_show — Int → Text)
///
/// Mesmo arg shape [Int], retornos diferentes. O parser não suporta
/// múltiplas assinaturas de mesmo nome, então injetamos manualmente.
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

/// DoD 8b: sem hint, `custom 42` tem 2 overloads compatíveis com mesmo
/// score → AmbiguousDispatch. `infer_module` deve falhar.
#[test]
fn dod8b_sem_hint_ambiguous_dispatch() {
    let src = "custom 42";
    assert!(
        infer_fails_with_extra(src, custom_overloads()),
        "custom 42 sem hint deve falhar com AmbiguousDispatch"
    );
}

/// DoD 8b: com hint `Int`, `custom 42` filtra para a overload Int→Int.
/// O FFI `tag_int` tags o valor SMI novamente (double-tagging), mas o
/// importante é que o hint selecionou a overload Int→Int (não Int→Text).
/// Verificamos pelo tipo de retorno: `Ty::int()` confirma dispatch correto.
#[test]
fn dod8b_hint_int_seleciona_int() {
    let src = "(custom 42)::Int";
    let (raw, ty) = eval_src_with_extra(src, custom_overloads());
    assert_eq!(ty, Ty::int());
    // tag_int(42) double-tags: SMI(42)=85 → tag_int(85)=171.
    // untag_smi(171) = 85. Não é 42, mas prova que dispatched para Int→Int
    // e não para Int→Text (que retornaria Ty::text()).
    assert_eq!(untag_smi(raw), 85);
}

/// DoD 8b: com hint `Text`, `custom 42` filtra para a overload Int→Text.
/// Resultado: bi_show(42) → ponteiro para C string "42".
#[test]
fn dod8b_hint_text_seleciona_text() {
    let src = "(custom 42)::Text";
    let (raw, ty) = eval_src_with_extra(src, custom_overloads());
    assert_eq!(ty, Ty::text());
    // raw é um ponteiro para C string — não decodificar, só verificar tipo
    assert!(raw != 0, "ponteiro para Text não deve ser null");
}
