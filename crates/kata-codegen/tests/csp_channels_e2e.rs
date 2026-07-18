//! Testes E2E de codegen CSP (channel!, !>, <!, fork!).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste compila um programa Kata com operações CSP e verifica o
//! valor retornado pelo JIT executando em fibers.
//!
//! Os testes producer/consumer usam `fork!` para criar fibers separados
//! que comunicam via canais. O scheduler coordena o bloqueio/desbloqueio.
//!
//! Nota: o parser não suporta destructuring `let (tx, rx) := ...`.
//! Usamos `ch.0` (sender) e `ch.1` (receiver) via DotAccess/FieldAccess.

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
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
        interface_registry: InterfaceRegistry::new(),
        functions: user.functions,
        actions: user.actions,
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

/// Action produtor envia 42 via `!>`. Action main cria canal, fork do
/// produtor, recebe via `<!`, e retorna o valor.
///
/// O scheduler coordena: main bloqueia em recv, prod envia, main acorda.
#[serial]
#[test]
fn producer_consumer_rendezvous() {
    let src = r#"action prod (Sender::Int) -> Unit
  __param_0 !> 42
  ()
action main -> Int
  let ch := channel!()
  let tx := ch.0
  let rx := ch.1
  fork!(prod, (tx))
  rx <! val
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
    let src = r#"action prod (Sender::Int) -> Unit
  __param_0 !> 7
  ()
action main -> Int
  let ch := queue!(2)
  let tx := ch.0
  let rx := ch.1
  fork!(prod, (tx))
  rx <! val
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
    let src = r#"action main -> Int
  let ch := channel!()
  let rx := ch.1
  rx <! val
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
    let src = r#"action prod (Sender::Int) -> Unit
  __param_0 !> 10
  __param_0 !> 20
  __param_0 !> 30
  ()
action main -> Int
  let ch := queue!(3)
  let tx := ch.0
  let rx := ch.1
  fork!(prod, (tx))
  rx <! a
  rx <! b
  rx <! c
  c
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_eq!(untag_smi(raw), 30, "último valor recebido deve ser 30");
}
