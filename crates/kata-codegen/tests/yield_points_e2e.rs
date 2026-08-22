//! Testes E2E de codegen de yield points (back-edge checks).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste compila um programa Kata com loops e verifica que:
//!
//! 1. Loops simples sem outras fibers funcionam (yield_check é no-op quando
//!    `HAS_READY_FIBER` é false — não suspende desnecessariamente).
//! 2. Fiber em loop pesado cede periodicamente; outras fibers executam
//!    durante o loop (head-of-line blocking resolvido).
//! 3. ForIn também cede — yield_check nos headers de List/Array/Range.
//!
//! Limitações do parser (pitfall #45):
//! - `let (tx, rx) := ...` suportado via destructuring de tupla.
//! - `()` para Unit (não `Unit` como Ident).
//! - Valores recebidos têm tipo `Var("T0")` — evitar operações aritméticas
//!   sobre o valor recebido; retorná-lo diretamente.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;
use serial_test::serial;

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
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Combina prelude + módulo do usuário (replica do driver com merge completo).
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

    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);

    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);

    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Sentinel de deadlock retornado por `kata_rt_run`.
use kata_rt::DEADLOCK_SENTINEL;

// ── Teste 1: loop simples sem outras fibers — yield_check é no-op ──

/// Loop de 1..100 sem outras fibers na run_queue. `HAS_READY_FIBER` é false,
/// então `kata_rt_yield_check` retorna sem suspender mesmo no slowpath.
/// Verifica que o yield point não quebra loops simples e o resultado está
/// correto (soma 1..100 = 5050).
#[serial]
#[test]
fn loop_simples_funciona() {
    let src = r#"action loop_simples => Int
  var acc := 0
  var i := 0
  loop
    i := + i 1
    match > i 100
      True: break
      False: acc := + acc i
  acc
loop_simples!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        5050,
        "soma 1..100 deve ser 5050 (yield_check não quebra loop simples)"
    );
}

// ── Teste 2: head-of-line blocking — loop pesado + canal ──

/// Action `loop_worker` faz loop pesado (1..5000) e envia a soma no canal.
/// Main fork o worker, recebe via `!>`, retorna o valor.
///
/// Com yield points, `loop_worker` cede CPU a cada YIELD_INTERVAL (1000)
/// iterações. Sem yield points, o loop monopolizaria a fiber até completar
/// — main esperaria todas as 5000 iterações antes de receber.
///
/// DoD: "Fiber em loop pesado cede periodicamente. Outras fibers
/// executam durante o loop."
///
/// Soma 1..5000 = 12502500.
#[serial]
#[test]
fn yield_loop_nao_bloqueia() {
    let src = r#"action loop_worker (tx::Sender::Int) => Unit
  var acc := 0
  var i := 0
  loop
    i := + i 1
    match > i 5000
      True: break
      False: acc := + acc i
  tx <! acc
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(loop_worker, (tx))
  rx !> val
  val
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(
        untag_smi(raw),
        12502500,
        "soma 1..5000 deve ser 12502500 (loop pesado com yield point)"
    );
}

// ── Teste 3: ForIn com yield point — Array ──

/// Action `forin_worker` faz `for x in {1 2 3 4 5}` (ForIn sobre Array) e
/// envia a soma no canal. Exercita o yield_check no header do ForIn-Array.
///
/// Soma 1+2+3+4+5 = 15.
#[serial]
#[test]
fn yield_forin_nao_bloqueia() {
    let src = r#"action forin_worker (tx::Sender::Int) => Unit
  var acc := 0
  for x in {1 2 3 4 5}
    acc := + acc x
  tx <! acc
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(forin_worker, (tx))
  rx !> val
  val
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(
        untag_smi(raw),
        15,
        "soma 1+2+3+4+5 deve ser 15 (ForIn com yield point)"
    );
}
