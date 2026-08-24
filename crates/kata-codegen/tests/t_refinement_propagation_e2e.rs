//! Testes E2E de refinement propagation — path conditions provam
//! ascriptions refinadas sobre não-literais em compile-time.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//!
//! Cenários testados:
//! 1. Guard direto → ascription provada (compile-time)
//! 2. Match sobre Boolean → ascription provada (compile-time)
//! 3. Guard contradiz predicado → erro compile-time (refutação)
//! 4. Sem path conditions → comportamento inalterado (pending/fallback)
//! 5. Match aninhado → facts compostos

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

fn infer_src(src: &str) -> Result<kata_inference::TypedModule, kata_diagnostics::MiddleError> {
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
    )
    .expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved)
}

fn infer_fails(src: &str) -> bool {
    infer_src(src).is_err()
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

// ── 1. Guard direto → ascription provada ──────────────────────────

/// Guard `> n 0` prova o predicado `> _ 0` de PositiveInt.
/// A ascription `n::PositiveInt` é validada em compile-time pelo Z3.
/// O braço False usa construtor (não contradiz).
#[test]
fn t_guard_prova_ascription_refined() {
    let src = r#"data (Int, > _ 0) as PositiveInt

action test_guard => PositiveInt
    let n := 5
    match (> n 0)
        Boolean::True: n::PositiveInt
        Boolean::False: 1::PositiveInt
test_guard!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Plain("PositiveInt".into())),
        "guard deve provar ascription e retornar PositiveInt"
    );
    assert_eq!(untag_smi(raw), 5);
}

// ── 2. Match Boolean → ascription provada ─────────────────────────

/// `match (> n 0): Boolean::True → n::PositiveInt` — o fact `> n 0`
/// é extraído do pattern Boolean::True e prova o predicado.
/// O braço False usa literal (não contradiz).
#[test]
fn t_match_boolean_prova_ascription() {
    let src = r#"data (Int, > _ 0) as PositiveInt

action test_match => PositiveInt
    let n := 7
    match (> n 0)
        Boolean::True: n::PositiveInt
        Boolean::False: 1::PositiveInt
test_match!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Plain("PositiveInt".into())),
        "match Boolean::True deve provar ascription"
    );
    assert_eq!(untag_smi(raw), 7);
}

// ── 3. Guard contradiz predicado → refutação compile-time ─────────

/// `<= n 0` contradiz `> _ 0`. O Z3 prova que o predicado é refutado
/// pelas path conditions. Deve ser erro compile-time.
#[test]
fn t_guard_refuta_ascription_refined() {
    let src = r#"data (Int, > _ 0) as PositiveInt

action test_refute => PositiveInt
    let n := 5
    match (<= n 0)
        Boolean::True: n::PositiveInt
        Boolean::False: n::PositiveInt
test_refute!()"#;
    assert!(
        infer_fails(src),
        "guard <= n 0 contradiz > _ 0, deve falhar em compile-time"
    );
}

// ── 4. Sem path conditions → rejeita não-literal (comportamento original) ─

/// Sem guard e sem match Boolean, path conditions estão vazias.
/// `n::PositiveInt` onde n é Ident é rejeitado — exige literal ou
/// construtor. Este é o comportamento original (pré-refinement-propagation).
#[test]
fn t_sem_path_conditions_rejeita_nao_literal() {
    let src = r#"data (Int, > _ 0) as PositiveInt

action test_no_pc => PositiveInt
    let n := 5
    n::PositiveInt
test_no_pc!()"#;
    // Deve falhar — sem path conditions, não-literal é rejeitado.
    assert!(
        infer_fails(src),
        "sem path conditions, ascription de não-literal deve falhar"
    );
}

// ── 5. Match aninhado → facts compostos ───────────────────────────

/// Match aninhado: braço interno `True` tem facts `[> n 0, > n 10]`.
/// Ascription `n::PositiveInt` é provada por `> n 0` (fact externo).
/// Os braços não-provados usam literal.
#[test]
fn t_match_aninhado_facts_compostos() {
    let src = r#"data (Int, > _ 0) as PositiveInt

action test_nested => PositiveInt
    let n := 15
    match (> n 0)
        Boolean::True:
            match (> n 10)
                Boolean::True: n::PositiveInt
                Boolean::False: 1::PositiveInt
        Boolean::False: 1::PositiveInt
test_nested!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Plain("PositiveInt".into())),
        "match aninhado deve propagar facts compostos"
    );
    assert_eq!(untag_smi(raw), 15);
}

// ── 6. Lambda com guard → ascription provada ──────────────────────

/// Lambda com guard direto: `> n 0:` prova `n::PositiveInt`.
/// O otherwise usa literal (não contradiz).
#[test]
fn t_lambda_guard_prova_ascription() {
    let src = r#"data (Int, > _ 0) as PositiveInt

classify :: Int => PositiveInt
lambda n:
    > n 0: n::PositiveInt
    otherwise: 1::PositiveInt

echo!(classify 5)"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit, "echo! retorna Unit");
}

// ── 7. Lambda guard refuta → erro compile-time ────────────────────

/// Lambda com guard `<= n 0:` contradiz `> _ 0`. Deve falhar.
/// O otherwise usa literal para não confundir (só o braço do guard é testado).
#[test]
fn t_lambda_guard_refuta_ascription() {
    let src = r#"data (Int, > _ 0) as PositiveInt

classify :: Int => PositiveInt
lambda n:
    <= n 0: n::PositiveInt
    otherwise: 1::PositiveInt

echo!(classify 5)"#;
    assert!(
        infer_fails(src),
        "lambda guard <= n 0 contradiz > _ 0, deve falhar em compile-time"
    );
}

// ── 8. Boolean::False → fact negado prova predicado ───────────────

/// `match (<= n 0): Boolean::False` → fact `not(<= n 0)` = `n > 0`.
/// Ascription `n::PositiveInt` é provada pela negação.
/// O braço True usa literal (n <= 0 contradiz > 0).
#[test]
fn t_boolean_false_fact_negado_prova() {
    let src = r#"data (Int, > _ 0) as PositiveInt

action test_false => PositiveInt
    let n := 5
    match (<= n 0)
        Boolean::True: 1::PositiveInt
        Boolean::False: n::PositiveInt
test_false!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Plain("PositiveInt".into())),
        "Boolean::False deve extrair fact negado e provar ascription"
    );
    assert_eq!(untag_smi(raw), 5);
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
