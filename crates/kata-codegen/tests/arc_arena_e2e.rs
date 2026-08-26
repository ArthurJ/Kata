//! Testes E2E do ARC Arena (Fio 16, Fase 7).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste compila um programa Kata com operações CSP que exercitam o
//! sistema ARC (alloc tracked, incref, decref, deallocation individual).
//!
//! Estes testes verificam:
//! 1. Valores compostos (List, Tuple) enviados por canal sobrevivem ao sender
//! 2. Primitivos (Int) continuam funcionando sem overhead de ARC
//! 3. Múltiplos sends/receives não corrompem memória

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;
use serial_test::serial;

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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Sentinel de deadlock retornado por `kata_rt_run`.
use kata_rt::DEADLOCK_SENTINEL;

// ═══════════════════════════════════════════════════════════════════
// Teste 1: Lista enviada por canal sobrevive ao sender (List = composto)
// ═══════════════════════════════════════════════════════════════════

/// Producer fork envia uma Lista `[1 2 3]` via canal e termina (fiber
/// destruído). Consumer (main) recebe a lista após o producer ter
/// terminado. Como List é tipo composto, é ARC-managed na root_arena.
/// Se o valor estivesse na fiber arena do sender, seria UAF.
///
/// O teste verifica que `head` da lista recebida = 1 (primeiro elemento)
/// sem crash, provando que a lista sobreviveu à morte do sender.
#[serial]
#[test]
fn lista_enviada_por_canal_sobrevive_sender() {
    let src = r#"action prod (tx::Sender::List::Int) => Unit
  tx <! [1 2 3]
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(prod, (tx))
  rx !> lst
  head lst
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(
        untag_smi(raw),
        1,
        "head da lista recebida deve ser 1 (lista sobreviveu ao sender)"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Teste 2: Tupla enviada por canal sobrevive ao sender
// ═══════════════════════════════════════════════════════════════════

/// Producer envia uma tupla `(10, 20)` via canal. Main recebe e acessa
/// o primeiro elemento (.0). Tupla é tipo composto → ARC-managed.
#[serial]
#[test]
fn tupla_enviada_por_canal_sobrevive_sender() {
    let src = r#"action prod (tx::Sender::Tuple::(Int, Int)) => Unit
  tx <! (10, 20)
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(prod, (tx))
  rx !> tpl
  tpl.0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(
        untag_smi(raw),
        10,
        "primeiro elemento da tupla recebida deve ser 10"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Teste 3: Primitivo (Int) por canal continua funcionando (sem ARC)
// ═══════════════════════════════════════════════════════════════════

/// Int enviado por canal não passa por ARC (é SMI-tagged, inline).
/// O teste verifica que primitivos continuam funcionando sem overhead.
#[serial]
#[test]
fn primitivo_int_por_canal_sem_arc() {
    let src = r#"action prod (tx::Sender::Int) => Unit
  tx <! 42
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(prod, (tx))
  rx !> v
  v
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "primitivo Int deve chegar intacto");
}

// ═══════════════════════════════════════════════════════════════════
// Teste 4: Múltiplos valores compostos por canal (queue bufferizada)
// ═══════════════════════════════════════════════════════════════════

/// Producer envia 3 listas via queue!(3). Consumer recebe as 3 e retorna
/// o head da última. Verifica que múltiplas alocações ARC-managed não
/// corrompem memória e FIFO é preservado.
#[serial]
#[test]
fn multiplos_listas_por_queue_bufferizada() {
    let src = r#"action prod (tx::Sender::List::Int) => Unit
  tx <! [10 20]
  tx <! [30 40]
  tx <! [50 60]
  ()
action main => Int
  let (tx, rx) := queue!(3)
  fork!(prod, (tx))
  rx !> a
  rx !> b
  rx !> c
  head c
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(
        untag_smi(raw),
        50,
        "head da terceira lista (FIFO) deve ser 50"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Teste 5: Lista com len — acessa dado após receber por canal
// ═══════════════════════════════════════════════════════════════════

/// Producer envia `[1 2 3 4 5]`. Consumer recebe e chama `len`.
/// Verifica que a lista recebida é funcional — não apenas o ponteiro
/// sobrevive, mas os dados internos (Cons chain) são acessíveis.
#[serial]
#[test]
fn lista_recebida_por_canal_len_retorna_5() {
    let src = r#"action prod (tx::Sender::List::Int) => Unit
  tx <! [1 2 3 4 5]
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(prod, (tx))
  rx !> lst
  len lst
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(untag_smi(raw), 5, "len da lista recebida deve ser 5");
}

// ═══════════════════════════════════════════════════════════════════
// Teste 6: Structured concurrency com valor composto — parent espera fork
// ═══════════════════════════════════════════════════════════════════

/// Worker fork computa uma lista `[100 200 300]` e envia. Parent
/// recebe e retorna `head`. O worker termina antes do parent consumir
/// (structured concurrency garante que o parent espera, mas o fiber
/// do worker é destruído antes do recv completar). Se a lista
/// estivesse na arena do worker, seria UAF.
#[serial]
#[test]
fn structured_concurrency_lista_composta() {
    let src = r#"action worker (tx::Sender::List::Int) => Unit
  tx <! [100 200 300]
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(worker, (tx))
  rx !> result
  head result
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(untag_smi(raw), 100, "head da lista do worker deve ser 100");
}
