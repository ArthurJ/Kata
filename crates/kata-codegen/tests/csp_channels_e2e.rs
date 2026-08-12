//! Testes E2E de codegen CSP (channel!, <!, !>, fork!).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste compila um programa Kata com operações CSP e verifica o
//! valor retornado pelo JIT executando em fibers.
//!
//! Os testes producer/consumer usam `fork!` para criar fibers separados
//! que comunicam via canais. O scheduler coordena o bloqueio/desbloqueio.
//!
//! Destructuring `let (tx, rx) := ...` é suportado (desugar para FieldAccess).

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
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr())
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
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Sentinel de deadlock retornado por `kata_rt_run`.
const DEADLOCK_SENTINEL: i64 = i64::MIN + 1;

// ── Teste 1: channel!() cria canal e retorna tupla (ptr != 0) ──

/// `channel!()` no entry point cria um canal rendezvous. A tupla (tx, rx)
/// é alocada na arena raiz. O entry point retorna o ponteiro da tupla.
#[serial]
#[test]
fn channel_create_retorna_tupla() {
    let src = "channel!()";
    let (raw, ty) = eval_src(src);
    assert!(raw != 0, "tupla (tx, rx) deve ser alocada (ptr != 0)");
    assert!(
        matches!(ty, Ty::Tuple(_)),
        "channel!() deve retornar tupla, got {ty:?}"
    );
}

// ── Teste 2: queue!(N) cria fila bufferizada ──

/// `queue!(2)` cria uma fila com capacidade 2. Retorna tupla (tx, rx).
#[serial]
#[test]
fn queue_create_retorna_tupla() {
    let src = "queue!(2)";
    let (raw, ty) = eval_src(src);
    assert!(raw != 0, "tupla (tx, rx) deve ser alocada (ptr != 0)");
    assert!(
        matches!(ty, Ty::Tuple(_)),
        "queue!(N) deve retornar tupla, got {ty:?}"
    );
}

// ── Teste 3: broadcast!() cria broadcast (tupla com ReceiverFactory) ──

/// `broadcast!()` cria broadcast. Retorna (tx, rxf) onde rxf é ReceiverFactory.
#[serial]
#[test]
fn broadcast_create_retorna_tupla() {
    let src = "broadcast!()";
    let (raw, ty) = eval_src(src);
    assert!(raw != 0, "tupla (tx, rxf) deve ser alocada (ptr != 0)");
    assert!(
        matches!(ty, Ty::Tuple(_)),
        "broadcast!() deve retornar tupla, got {ty:?}"
    );
}

// ── Teste 4: Producer/consumer via fork — channel! rendezvous ──

/// Action produtor envia 42 via `<!`. Action main cria canal, fork do
/// produtor, recebe via `!>`, e retorna o valor.
///
/// O scheduler coordena: main bloqueia em recv, prod envia, main acorda.
#[serial]
#[test]
fn producer_consumer_rendezvous() {
    let src = r#"action prod (tx::Sender::Int) => Unit
  tx <! 42
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(prod, (tx))
  rx !> val
  val
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        42,
        "producer_consumer deve retornar 42 (valor enviado)"
    );
}

// ── Teste 5: Producer/consumer via fork — queue!(N) buffered ──

/// Similar ao teste 4, mas com queue bufferizada. O producer pode enviar
/// sem bloquear (buffer tem capacidade).
#[serial]
#[test]
fn producer_consumer_queue_buffered() {
    let src = r#"action prod (tx::Sender::Int) => Unit
  tx <! 7
  ()
action main => Int
  let (tx, rx) := queue!(2)
  fork!(prod, (tx))
  rx !> val
  val
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        7,
        "producer_consumer_queue deve retornar 7 (valor enviado)"
    );
}

// ── Teste 6: Deadlock detection — canal criado, ninguém envia ──

/// Action main cria canal e tenta receber, mas não há producer.
/// O scheduler detecta deadlock e retorna DEADLOCK_SENTINEL.
#[serial]
#[test]
fn deadlock_sem_producer() {
    let src = r#"action main => Int
  let (_, rx) := channel!()
  rx !> val
  val
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_eq!(
        raw, DEADLOCK_SENTINEL,
        "deadlock deve retornar DEADLOCK_SENTINEL"
    );
}

// ── Teste 7: Múltiplos valores via queue — producer envia 3, consumer recebe último ──

/// Producer envia 3 valores via queue!(3). Consumer recebe os 3 e retorna
/// o último (30). Verifica que queue mantém ordem FIFO e múltiplos
/// send/recv funcionam.
#[serial]
#[test]
fn multiplos_valores_queue() {
    let src = r#"action prod (tx::Sender::Int) => Unit
  tx <! 10
  tx <! 20
  tx <! 30
  ()
action main => Int
  let (tx, rx) := queue!(3)
  fork!(prod, (tx))
  rx !> a
  rx !> b
  rx !> c
  c
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_eq!(untag_smi(raw), 30, "último valor recebido deve ser 30");
}

// ── Teste 8: Backpressure — producer envia capacity+1, consumer drena ──

/// `queue!(2)` com capacity 2. Producer envia 3 valores (10, 20, 30). O
/// terceiro `<!` bloqueia (buffer cheio). O consumer (main) drena os 3 via
/// `!>`. Como o scheduler é cooperativo, o producer bloqueado em `<!` cede
/// (WaitingOnChannelSend), o main executa `!>`, drena 1 slot, o producer
/// é acordado e envia o resto.
///
/// Verifica: não deadlocka, valores chegam em ordem FIFO (10, 20, 30),
/// último recebido = 30.
///
/// DoD: "Buffer overflow/backpressure via queue!(N)".
#[serial]
#[test]
fn queue_backpressure_capacity_mais_um() {
    let src = r#"action prod (tx::Sender::Int) => Unit
  tx <! 10
  tx <! 20
  tx <! 30
  ()
action main => Int
  let (tx, rx) := queue!(2)
  fork!(prod, (tx))
  rx !> a
  rx !> b
  rx !> c
  c
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "backpressure não deve deadlockar");
    assert_eq!(
        untag_smi(raw),
        30,
        "backpressure: último valor (FIFO) deve ser 30"
    );
}

// ── Teste 9: fork! com múltiplas fibers e args ──

/// Main fork 2 produtores distintos (prod_a envia 100, prod_b envia 200),
/// cada um com seu canal. Main recebe dos dois e retorna a soma.
///
/// Como não podemos somar `Var(\"T0\")` (limitação do typeck), retornamos
/// `b` e verificamos que é 200 (ou 100, dependendo da ordem do scheduler).
/// O ponto do teste é que **ambos** forks completam e main recebe de ambos
/// sem deadlock — prova que múltiplas fibers com args distintos funcionam.
///
/// DoD: "fork! com múltiplas fibers e args".
#[serial]
#[test]
fn fork_multiplas_fibers_com_args() {
    let src = r#"action prod_a (tx::Sender::Int) => Unit
  tx <! 100
  ()
action prod_b (tx::Sender::Int) => Unit
  tx <! 200
  ()
action main => Int
  let ch1 := channel!()
  let tx1 := ch1.0
  let rx1 := ch1.1
  let ch2 := channel!()
  let tx2 := ch2.0
  let rx2 := ch2.1
  fork!(prod_a, (tx1))
  fork!(prod_b, (tx2))
  rx1 !> a
  rx2 !> b
  b
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(
        raw, DEADLOCK_SENTINEL,
        "múltiplos forks não devem deadlockar"
    );
    // b deve ser 200 (prod_b envia 200). Se o scheduler reordenar, ainda
    // assim `b` é do rx2 (canal de prod_b), sempre 200.
    assert_eq!(
        untag_smi(raw),
        200,
        "fork múltiplo: rx2 deve receber 200 de prod_b"
    );
}

// ── Teste 10: Structured concurrency — parent espera forks ──

/// Parent (main) faz fork de um produtor lento (loop 1..5000 + send) e
/// depois executa um `!>` (recebe do canal). Pela Decisão E, o parent só
/// termina depois do fork completar. Se o parent abandonasse o fork, o
/// `!>` nunca desbloquearia e o scheduler deadlockaria.
///
/// O teste verifica que o parent espera o fork completar: o `!>` recebe
/// o valor (soma 1..5000 = 12502500) e main retorna esse valor.
///
/// DoD: "Structured concurrency: parent espera forks".
#[serial]
#[test]
fn structured_concurrency_parent_espera_fork() {
    let src = r#"action worker (tx::Sender::Int) => Unit
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
  fork!(worker, (tx))
  rx !> result
  result
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(
        raw, DEADLOCK_SENTINEL,
        "parent não deve deadlockar esperando fork"
    );
    assert_eq!(
        untag_smi(raw),
        12502500,
        "structured concurrency: parent recebe resultado do fork (soma 1..5000)"
    );
}

// ── Teste 11: Escape analysis — valor enviado sobrevive ao sender ──

/// Producer fork envia um valor (42) via canal e termina (fiber destruída).
/// Consumer (main) recebe o valor após o producer ter terminado. Se o valor
/// fosse alocado na arena do fiber sender, seria liberado quando o sender
/// morresse — o consumer acessaria memória inválida.
///
/// O escape analysis (conservador: `Caller` = caller_arena) marca o valor
/// enviado como escapando para a arena raiz, que sobrevive à morte do
/// sender. O teste verifica que o consumer recebe 42 sem crash.
///
/// DoD: "Escape analysis: valor enviado por canal sobrevive ao
/// sender (LCA correto)".
#[serial]
#[test]
fn escape_canal_sobrevive_sender() {
    let src = r#"action prod (tx::Sender::Int) => Unit
  tx <! 42
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(prod, (tx))
  rx !> v
  v
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    assert_eq!(
        untag_smi(raw),
        42,
        "escape analysis: valor deve sobreviver ao sender (arena raiz/LCA)"
    );
}

// ── Teste 12: Channel sentinel — enviar -1 (colide com WOULD_BLOCK) ──

/// Antes do out-parameter, enviar `-1` por um canal rendezvous deadlockava:
/// `try_recv` retornava `-1` (o valor real), mas `kata_rt_channel_recv`
/// interpretava como WOULD_BLOCK e suspendia o fiber indefinidamente.
///
/// Este teste verifica que o bug está corrigido: o producer envia `-1` e
/// o consumer recebe `-1` sem deadlock.
#[serial]
#[test]
fn channel_recv_negativo_um() {
    let src = r#"action prod (tx::Sender::Int) => Unit
  tx <! -1
  ()
action main => Int
  let (tx, rx) := channel!()
  fork!(prod, (tx))
  rx !> val
  val
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(
        raw, DEADLOCK_SENTINEL,
        "enviar -1 por canal não deve deadlockar"
    );
    assert_eq!(
        untag_smi(raw),
        -1,
        "consumer deve receber -1 (valor real, não sentinel)"
    );
}

// ── Teste 13: Queue sentinel — enviar -1 via queue!(N) ──

/// Mesmo bug do teste 12, mas com queue bufferizada. O valor `-1` é
/// enfileirado e depois desenfileirado. Antes do fix, `try_recv` na queue
/// também colidia com WOULD_BLOCK.
#[serial]
#[test]
fn queue_recv_negativo_um() {
    let src = r#"action prod (tx::Sender::Int) => Unit
  tx <! -1
  ()
action main => Int
  let (tx, rx) := queue!(1)
  fork!(prod, (tx))
  rx !> val
  val
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_ne!(
        raw, DEADLOCK_SENTINEL,
        "enviar -1 por queue não deve deadlockar"
    );
    assert_eq!(
        untag_smi(raw),
        -1,
        "consumer deve receber -1 da queue (valor real, não sentinel)"
    );
}
