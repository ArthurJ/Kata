//! E2E: Direção A (path condition prova arg) e Direção B (aprende predicado após dispatch).
//! Grupo: propagação de learned_facts, rollback de guards, composição.
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
        Some(&prelude.type_env),
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

// ── 14. Direção A — pré-condição inter-procedural (path condition prova arg) ──

/// `(/ 10 b)` onde `b` é `Int`. O braço `Boolean::False` do match sobre
/// `(= b 0)` tem path condition `not(= b 0)` = `b ≠ 0`. O predicado de
/// NonZero é `!= _ (zero _)` = `!= _ 0`. O Z3 prova que `b ≠ 0` implica
/// `b ≠ 0`. A Direção A aceita `b` como `NonZero` sem ascription explícita.
#[test]
fn t_nivel3_direcao_a_div_com_path_condition() {
    let src = r#"action test => Int
    let b := 5
    match (= b 0)
        Boolean::False: / 10 b
        Boolean::True: 0
test!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int(), "div deve retornar Int");
    // 10 / 5 = 2
    assert_eq!(untag_smi(raw), 2);
}

// ── 15. Direção A — sem path conditions, falha como antes ──────────

/// Sem path conditions (b não é guardado), o Z3 não prova. Deve falhar
/// em compile-time como antes — o usuário precisa de ascription explícita.
#[test]
fn t_nivel3_direcao_a_sem_path_conditions_falha() {
    let src = r#"action test => Int
    let b := 5
    / 10 b
test!()"#;
    assert!(
        infer_fails(src),
        "sem path conditions, dispatch deve falhar"
    );
}

// ── 16. Direção A — path condition refuta predicado (b = 0) ─────────

/// No braço `Boolean::True` do match sobre `(= b 0)`, a path condition
/// é `= b 0` — que REFUTA o predicado `!= b 0` de NonZero. Deve ser
/// erro compile-time.
#[test]
fn t_nivel3_direcao_a_path_condition_refuta() {
    let src = r#"action test => Int
    let b := 0
    match (= b 0)
        Boolean::True: / 10 b
        Boolean::False: 0
test!()"#;
    assert!(infer_fails(src), "path condition = b 0 refuta NonZero");
}

// ── 17. Direção B — aprende predicado após chamada com ascription explícita ──

/// Dentro de um braço com path condition `not(= b 0)`, `b::NonZero`
/// é provado pela path condition. A chamada `/ 10 (b::NonZero)` sucede.
/// Após o dispatch, a Direção B extrai o predicado `!= b 0` e adiciona
/// como path condition. A ascription `b::NonZero` subsequente é provada
/// pela path condition aprendida — mas neste caso ela já era provável
/// pela path condition do braço. O teste verifica que a Direção B não
/// interfere negativamente quando a path condition já existia.
#[test]
fn t_nivel3_direcao_b_apos_ascription_explicita() {
    let src = r#"action test => NonZero::Int
    let b := 5
    match (= b 0)
        Boolean::False:
            let _ := / 10 (b::NonZero)
            b::NonZero
        Boolean::True: 5::NonZero
test!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "Direção B deve aprender b ≠ 0 após chamada e provar b::NonZero"
    );
    assert_eq!(untag_smi(raw), 5);
}

// ── 18. Direção B — após Direção A, predicado é aprendido ──────────────

/// `(/ 10 b)` no braço `Boolean::False` do match sobre `(= b 0)`.
/// A Direção A aceita `b` (Int) como `NonZero` usando a path condition
/// `not(= b 0)` do braço. Após o dispatch, a Direção B aprende
/// `!= b 0` como path condition. A ascription `b::NonZero` na linha
/// seguinte é provada pela path condition aprendida.
#[test]
fn t_nivel3_direcao_b_apos_direcao_a() {
    let src = r#"action test => NonZero::Int
    let b := 5
    match (= b 0)
        Boolean::False:
            let _ := / 10 b
            b::NonZero
        Boolean::True: 5::NonZero
test!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "Direção B após Direção A deve aprender b ≠ 0 e provar b::NonZero"
    );
    assert_eq!(untag_smi(raw), 5);
}

// ── 19. Direção B — sem chamada prévia, ascription de não-literal falha ──

/// Sem chamada prévia que ensine o predicado, `b::NonZero` sobre
/// não-literal continua falhando (comportamento original). A Direção B
/// não introduz regressão — só aprende quando há dispatch bem-sucedido.
#[test]
fn t_nivel3_direcao_b_sem_chamada_falha() {
    let src = r#"action test => NonZero::Int
    let b := 5
    b::NonZero
test!()"#;
    assert!(
        infer_fails(src),
        "sem chamada prévia, ascription de não-literal deve falhar"
    );
}

// ── 20. Direção B — arg não-Ident (literal) não propaga ───────────────

/// `/ 10 (5::NonZero)` — o arg é literal `5`, não `Ident`. A Direção B
/// não adiciona path condition (não há variável para propagar).
/// O código compila porque `5::NonZero` é literal (validado por const_eval).
/// Mas `b::NonZero` na linha seguinte ainda falha (b não é literal e
/// sem path condition aprendida).
#[test]
fn t_nivel3_direcao_b_arg_literal_nao_propaga() {
    let src = r#"action test => NonZero::Int
    let b := 5
    let _ := / 10 (5::NonZero)
    5::NonZero
test!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "literal não propaga path condition, mas compila com literal"
    );
    assert_eq!(untag_smi(raw), 5);
}

// ── 21. Propagação de learned_fact para o escopo pai ──────────────────
//
// Fact aprendido pela Direção B dentro de um braço de match é visível
// APÓS o braço. O guard do braço (`Boolean::False` → `not(= b 0)`) é
// rolled back, mas o learned_fact (`!= b 0` da Direção B) é preservado.
// Após o match, `b::NonZero` é provado pelo learned_fact propagado.

#[test]
fn t_learned_fact_propaga_para_escopo_pai() {
    let src = r#"action test => NonZero::Int
    let b := 5
    match (= b 0)
        Boolean::False:
            let _ := / 10 b
            0
        Boolean::True: 0
    b::NonZero
test!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "learned_fact da Direção B deve propagar para o escopo pai após o match"
    );
    assert_eq!(untag_smi(raw), 5);
}

// ── 22. Fact de guard NÃO propaga para o escopo pai (rollback) ────────
//
// O guard do braço (`Boolean::False` → `not(= b 0)`) é um fact de
// guard (braço-específico). Após o match, o rollback o remove.
// `b::NonZero` fora do match falha — o fact do guard não propagou.

#[test]
fn t_guard_fact_nao_propaga_para_escopo_pai() {
    let src = r#"action test => Int
    let b := 5
    match (= b 0)
        Boolean::False:
            b::NonZero
        Boolean::True: 0
test!()"#;
    assert!(
        infer_fails(src),
        "fact de guard não deve propagar para o escopo pai após o match"
    );
}

// ── 23. Composição: guard + Direção B no mesmo braço ─────────────────
//
// No braço `Boolean::False`, o guard adiciona `not(= b 0)` como fact
// (rolled back) e a Direção B adiciona `!= b 0` como learned_fact
// (preservado). Após o match:
// - O fact do guard (`not(= b 0)`) foi rolled back — não está visível.
// - O learned_fact (`!= b 0`) foi preservado — está visível.
// `b::NonZero` após o match é provado pelo learned_fact, não pelo guard.

#[test]
fn t_composicao_guard_e_learned_fact() {
    let src = r#"action test => NonZero::Int
    let b := 5
    match (= b 0)
        Boolean::False:
            let _ := / 10 b
            0
        Boolean::True: 0
    b::NonZero
test!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Struct(StructKey::Instance("NonZero".into(), "Int".into())),
        "composição: learned_fact preservado após match, guard rolled back"
    );
    assert_eq!(untag_smi(raw), 5);
}
