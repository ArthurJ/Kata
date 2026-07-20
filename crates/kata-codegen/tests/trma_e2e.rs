//! Testes E2E de codegen de TRMA (Tail Recursion Modulo Associativity).
//!
//! `soma 1000000` sem TRMA causa stack overflow. Com TRMA, executa normalmente
//! via recursão de cauda com acumulador.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
use kata_core::ty::Ty;
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

/// Inferência sem codegen — para inspecionar a TAST.
fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("infer deve succeed")
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

// ── Teste E2E: soma recursiva com TRMA (adição) ─────────────────────

#[test]
fn soma_trma_e2e() {
    let src = r#"soma :: Int => Int
    lambda n: match n
        0: 0
        otherwise: + n (soma (- n 1))

soma 1000000"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    // 1000000 * 1000001 / 2 = 500000500000
    assert_eq!(untag_smi(raw), 500000500000);
}

// ── Teste E2E: fatorial recursivo com TRMA (multiplicação) ──────────

#[test]
fn fatorial_trma_e2e() {
    let src = r#"fat :: Int => Int
    lambda n: match n
        0: 1
        otherwise: * n (fat (- n 1))

fat 20"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    // 20! = 2432902008176640000
    assert_eq!(untag_smi(raw), 2432902008176640000);
}

// ── Teste: TRMA não ativa em recursão mútua ─────────────────────────

#[test]
fn recursao_mutua_nao_trma() {
    let src = r#"is_even :: Int => Boolean
    lambda n: match n
        0: Boolean::True
        otherwise: is_odd (- n 1)

is_odd :: Int => Boolean
    lambda n: match n
        0: Boolean::False
        otherwise: is_even (- n 1)

is_even 10"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Sum("Boolean".into()));
    // 10 é par → True. Boolean::True = 1, Boolean::False = 0 (sem SMI tag).
    assert_eq!(raw, 1);
}

// ── Teste: TAST inspection — soma_acc aparece após optimize ────────

#[test]
fn trma_cria_funcao_acc() {
    let src = r#"soma :: Int => Int
    lambda n: match n
        0: 0
        otherwise: + n (soma (- n 1))

soma 5"#;
    let typed = infer_src(src);
    // Antes do optimize: `soma` existe entre as funções (prelude + user).
    // Filtra só as funções do usuário (não-prelude) para inspecionar.
    let user_fns: Vec<_> = typed
        .functions
        .iter()
        .filter(|f| f.name == "soma")
        .collect();
    assert_eq!(user_fns.len(), 1, "deve ter exatamente 1 função `soma`");
    assert_eq!(user_fns[0].name, "soma");

    let typed = monomorphize(typed);
    let typed = optimize(typed);
    // Sem tree_shake — este teste inspeciona a TAST após optimize,
    // não executa codegen. Tree shaking removeria `soma` (não alcançada
    // após TRMA rewrite) e quebraria a asserção de estrutura.
    let typed = typed.inner;
    // Após optimize: existe `soma` (rewritten) e `soma_acc` (nova)
    // entre as funções (prelude + user). Filtra só as relevantes.
    let user_fns: Vec<_> = typed
        .functions
        .iter()
        .filter(|f| f.name == "soma" || f.name == "soma_acc")
        .collect();
    assert_eq!(user_fns.len(), 2, "deve ter soma e soma_acc");
    let names: Vec<&str> = user_fns.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"soma"));
    assert!(names.contains(&"soma_acc"));

    // soma_acc deve ter 2 parâmetros (n, acc)
    let acc_fn = typed
        .functions
        .iter()
        .find(|f| f.name == "soma_acc")
        .expect("soma_acc deve existir");
    assert_eq!(acc_fn.param_types.len(), 2);
}

// ── Teste: TRMA não ativa quando não há operador associativo ────────

#[test]
fn trma_nao_ativa_sem_associativo() {
    // `-` não é associativo — TRMA não deve ativar
    let src = r#"sub :: Int => Int
    lambda n: match n
        0: 0
        otherwise: - n (sub (- n 1))

sub 5"#;
    let typed = infer_src(src);
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    // Sem TRMA: só existe `sub`, nenhuma `sub_acc`
    assert_eq!(typed.functions.len(), 1);
    assert_eq!(typed.functions[0].name, "sub");
}
