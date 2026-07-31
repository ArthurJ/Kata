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

use kata_codegen::jit_eval;
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

// ── DoD 27: assert!(True, "msg") retorna Unit (não aborta) ──────────

/// DoD 27: `assert!(Boolean::True, "x deve ser positivo")` não aborta.
/// Retorna Unit. O desugar produz `match Boolean::True { True: Unit, False: panic!(msg) }`.
/// O braço True é tomado — retorna Unit. panic!(msg) não é avaliado.
#[test]
fn assert_true_retorna_unit() {
    let src = "action valida => Unit\n    assert!(Boolean::True, \"x deve ser positivo\")\n    echo!(\"ok\")\nvalida!()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit, "assert!(True) deve retornar Unit");
    assert_eq!(raw, 0, "Unit é 0");
}

/// DoD 27: `assert!(Boolean::True)` sem msg — não aborta.
/// Desugar: `match Boolean::True { True: Unit, False: panic!("assertion failed") }`.
#[test]
fn assert_true_sem_msg() {
    let src = "action valida => Unit\n    assert!(Boolean::True)\n    echo!(\"ok\")\nvalida!()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── DoD 26: panic!("msg") aborta — não testável via eval_src ─────────
//
// panic! chama std::process::exit(1) que mata o processo de teste inteiro.
// Para validar manualmente:
//   echo 'action crash => Unit
//       panic!("estado impossivel")
//   crash!()' > /tmp/test_panic.kata
//   cargo run --bin kata -- run /tmp/test_panic.kata
//   echo $?  → deve ser != 0

/// DoD 26: panic!("msg") aborta. Teste manual — #[ignore] porque mata o runner.
#[test]
#[ignore = "panic! chama exit(1) — mata o runner de testes. Validar via `cargo run --bin kata -- run <file>`"]
fn panic_aborta_com_mensagem() {
    let src = "action crash => Unit\n    panic!(\"estado impossivel\")\ncrash!()";
    let (raw, _) = eval_src(src);
    // Se chegou aqui, panic! não abortou — bug.
    let _ = raw;
    panic!("panic! deveria ter abortado");
}

/// DoD 27: assert!(False, "msg") aborta. Teste manual — #[ignore] mesmo motivo.
#[test]
#[ignore = "assert!(False) desugara para panic! que chama exit(1). Validar via subprocess."]
fn assert_false_aborta() {
    let src = "action valida => Unit\n    assert!(Boolean::False, \"x deve ser positivo\")\n    echo!(\"nao chega\")\nvalida!()";
    let (raw, _) = eval_src(src);
    let _ = raw;
    panic!("assert!(False) deveria ter abortado");
}
