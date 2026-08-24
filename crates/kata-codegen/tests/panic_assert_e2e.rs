//! Testes E2E de codegen de panic!/assert!.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Valida DoD 26-27: panic! aborta com mensagem, assert!(cond, "msg") verifica
//! condição e panic se falsa.
//!
//! Testes de abort (panic!/assert!(False)) não podem usar eval_src porque
//! panic! chama std::process::exit(1) que mata o runner de testes inteiro.
//! Esses testes ficam #[ignore] com nota — para validar manualmente, rodar
//! `cargo run --bin kata -- run <file>` e verificar exit code != 0.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
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
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
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
        type_graph: prelude.type_graph.clone(),
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

// ── DoD 27: assert!(True, "msg") retorna Unit (não aborta) ──────────

/// DoD 27: `assert!(True, "x deve ser positivo")` não aborta.
/// Retorna Unit. O desugar produz `match True { True: Unit, False: panic!(msg) }`.
/// O braço True é tomado — retorna Unit. panic!(msg) não é avaliado.
#[test]
fn assert_true_retorna_unit() {
    let src = "action valida => Unit\n    assert!(True, \"x deve ser positivo\")\n    echo!(\"ok\")\nvalida!()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit, "assert!(True) deve retornar Unit");
    assert_eq!(raw, 0, "Unit é 0");
}

/// DoD 27: `assert!(True)` sem msg — não aborta.
/// Desugar: `match True { True: Unit, False: panic!("assertion failed") }`.
#[test]
fn assert_true_sem_msg() {
    let src = "action valida => Unit\n    assert!(True)\n    echo!(\"ok\")\nvalida!()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── DoD 26-27: panic!/assert!(False) abortam — testados via subprocess ──
//
// panic! chama std::process::exit(1) que mata o processo de teste inteiro.
// Os testes de abort estão em `kata-driver/tests/panic_subprocess_e2e.rs`
// e executam `kata run` num processo filho isolado, verificando exit code
// e stderr. Os testes não-abortantes (assert!(True) → Unit) continuam aqui
// usando eval_src in-process.
