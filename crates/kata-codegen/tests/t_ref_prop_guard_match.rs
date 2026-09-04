//! E2E: path conditions provam ascriptions refinadas em compile-time.
//! Grupo: guards diretos, match Boolean, refutação, match aninhado, lambdas.
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

/// `<= n 0` contradiz `> _ 0` com n NÃO-LITERAL (param). O Z3 prova
/// que o predicado é refutado pelas path conditions no braço True
/// (fact `<= n 0` ∧ ¬`> n 0` é SAT). Deve ser erro compile-time.
///
/// NOTA (seeding de let-bindings): com `let n := <literal>`, o Z3
/// sabe o valor e o braço contraditório vira dead code — a prova é
/// vacuously true e o programa COMPILA (correto em runtime). A
/// refutação genuína só aparece com valor não conhecido (param).
#[test]
fn t_guard_refuta_ascription_refined() {
    let src = r#"data (Int, > _ 0) as PositiveInt

action test_refute (n::Int) => PositiveInt
    match (<= n 0)
        Boolean::True: n::PositiveInt
        Boolean::False: n::PositiveInt
test_refute!(5)"#;
    assert!(
        infer_fails(src),
        "guard <= n 0 contradiz > _ 0, deve falhar em compile-time"
    );
}

/// `let n := 5` (literal) + guard contraditório: o Z3 sabe `n = 5`,
/// o fact `<= 5 0` é falso — braço True é INALCANÇÁVEL (dead code).
/// A prova por vacuous truth é válida: o programa nunca viola o
/// predicado em runtime (sempre cai no False, onde `5 > 0` prova).
/// Comportamento novo do seeding de let-bindings (2026-08-29): onde
/// antes o compilador rejeitava um programa correto (falso positivo),
/// agora aceita com prova.
#[test]
fn t_guard_contraditorio_com_literal_e_dead_branch() {
    let src = r#"data (Int, > _ 0) as PositiveInt

action test_dead => PositiveInt
    let n := 5
    match (<= n 0)
        Boolean::True: n::PositiveInt
        Boolean::False: n::PositiveInt
test_dead!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Plain("PositiveInt".into())),
        "braço True é dead code (n=5 faz <= 5 0 falso); programa correto deve compilar"
    );
    assert_eq!(untag_smi(raw), 5);
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
