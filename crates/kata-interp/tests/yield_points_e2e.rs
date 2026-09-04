//! Testes E2E: Yield cooperativo em loops do interpretador (A4).
//!
//! Pipeline: lex → parse → resolve → infer → monomorph → optimize → interpret.
//! Verifica que o interpretador cede CPU cooperativamente em loops,
//! permitindo que outras fibers executem (paridade com o codegen JIT).
//!
//! Espelha os testes de `yield_points_e2e.rs` do codegen.

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_interp::{InterpError, interpret_with_registry};
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_rt::Runtime;

/// Sentinel de deadlock retornado por `kata_rt_run`.
use kata_rt::DEADLOCK_SENTINEL;

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

/// Roda o pipeline completo no interpretador e retorna (raw, ty).
fn interp_src(src: &str) -> Result<(i64, Ty), InterpError> {
    let src = src.to_string();
    let handle = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let tokens = lex(&src).expect("lex deve succeed");
            let module = parse(tokens).expect("parse deve succeed");
            let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
            let user = resolve(&module).expect("resolve deve succeed");
            let resolved = merge_resolved(prelude, user);
            let typed = infer_module(&module, &resolved).expect("infer deve succeed");
            let enum_registry = resolved.enum_registry.clone();
            let typed = monomorphize(typed);
            let typed = optimize(typed);
            let typed = typed.inner;
            let rt = Box::new(Runtime::new());
            let rt_ptr = Box::into_raw(rt) as i64;
            let result = interpret_with_registry(typed, rt_ptr, enum_registry);
            std::mem::forget(unsafe { Box::from_raw(rt_ptr as *mut Runtime) });
            result.map(|r| (r.raw, r.ty))
        })
        .unwrap();
    handle.join().unwrap()
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Loop simples sem outras fibers — yield_check é no-op (não há Suspend
/// ativo nem has_ready_fibers). Soma 1..100 = 5050.
#[test]
fn loop_simples_funciona_interp() {
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
    let (raw, ty) = interp_src(src).expect("interp deve succeed");
    assert_eq!(ty, Ty::Prim(kata_core::ty::PrimTy::Int));
    assert_eq!(untag_smi(raw), 5050, "soma 1..100 deve ser 5050");
}

/// Loop pesado (1..5000) + fork! + canal. Sem yield_check, o loop
/// monopolizaria a fiber e o main deadlockaria esperando o canal.
/// Com yield_check a cada 1000 iterações, o loop cede e o main recebe.
/// Soma 1..5000 = 12502500.
#[test]
fn yield_loop_nao_bloqueia_interp() {
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
    let (raw, ty) = interp_src(src).expect("interp deve succeed");
    assert_eq!(ty, Ty::Prim(kata_core::ty::PrimTy::Int));
    assert_ne!(
        raw, DEADLOCK_SENTINEL,
        "não deve deadlockar (yield cooperativo)"
    );
    assert_eq!(untag_smi(raw), 12502500, "soma 1..5000 deve ser 12502500");
}

/// ForIn com fork! + canal. ForIn também cede via yield_check.
/// Soma 1+2+3+4+5 = 15.
#[test]
fn yield_forin_nao_bloqueia_interp() {
    let src = r#"action forin_worker (tx::Sender::Int) => Unit
  var acc := 0
  for x in [1 2 3 4 5]
    acc := + acc x
  tx <! acc
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(forin_worker, (tx))
  rx !> val
  val
main!()"#;
    let (raw, ty) = interp_src(src).expect("interp deve succeed");
    assert_eq!(ty, Ty::Prim(kata_core::ty::PrimTy::Int));
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(untag_smi(raw), 15, "soma 1+2+3+4+5 deve ser 15");
}

/// ForIn sobre lista longa (1..1000) sem fork! — yield_check é no-op
/// (sem scheduler ativo). Soma 1..1000 = 500500.
#[test]
fn forin_simples_funciona_interp() {
    let src = r#"action forin_simples => Int
  var acc := 0
  for x in [1 2 3 4 5 6 7 8 9 10]
    acc := + acc x
  acc
forin_simples!()"#;
    let (raw, ty) = interp_src(src).expect("interp deve succeed");
    assert_eq!(ty, Ty::Prim(kata_core::ty::PrimTy::Int));
    assert_eq!(untag_smi(raw), 55, "soma 1..10 deve ser 55");
}
