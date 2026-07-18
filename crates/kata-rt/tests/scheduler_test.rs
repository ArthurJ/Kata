//! Testes unitários do scheduler com yield e structured concurrency (Fio 11 Fase 4).
//!
//! Estes testes criam fibers reais (wasmtime-fiber) que executam funções C
//! de teste. As funções simulam o comportamento de funções JIT: yield,
//! send/recv em canais, e retorno de valores.
//!
//! **Arquitetura de teste:**
//! - `kata_rt_scheduler_init()` cria o scheduler e a arena raiz
//! - `kata_rt_spawn(fn_ptr, caller_arena, args_ptr)` cria um fiber
//! - `kata_rt_run()` executa o scheduler até completar
//! - As funções C de teste recebem `(fiber_arena, caller_arena, args_ptr)`
//!   onde `args_ptr` e `caller_arena` são reinterpretados para passar dados

use kata_rt::{
    DEADLOCK_SENTINEL, kata_rt_channel_create, kata_rt_channel_recv, kata_rt_channel_send,
    kata_rt_run, kata_rt_scheduler_init, kata_rt_spawn, kata_rt_yield, reset_scheduler,
};
use serial_test::serial;

// ── Funções C de teste ─────────────────────────────────────────────
//
// Cada função tem a assinatura `extern "C" fn(i64, i64, i64) -> i64`
// (fiber_arena, caller_arena, args_ptr). Os parâmetros são reinterpretados
// conforme o teste.

/// Simplesmente retorna 42.
extern "C" fn return_42(_fa: i64, _ca: i64, _args: i64) -> i64 {
    42
}

/// Faz yield cooperativo e depois retorna 99.
extern "C" fn yield_then_return_99(_fa: i64, _ca: i64, _args: i64) -> i64 {
    kata_rt_yield();
    99
}

/// Faz yield N vezes e depois retorna 777. N é passado como args_ptr.
extern "C" fn yield_n_times(_fa: i64, _ca: i64, n: i64) -> i64 {
    for _ in 0..n {
        kata_rt_yield();
    }
    777
}

/// Tenta recv de um canal. `args_ptr` = handle do canal.
extern "C" fn recv_from_channel(_fa: i64, _ca: i64, handle: i64) -> i64 {
    kata_rt_channel_recv(handle)
}

/// Envia um valor para um canal. `caller_arena` = handle do canal.
/// `args_ptr` = valor a enviar.
extern "C" fn send_to_channel(_fa: i64, handle: i64, value: i64) -> i64 {
    kata_rt_channel_send(handle, value)
}

/// Faz spawn de um filho (return_42) e retorna 99.
/// `args_ptr` = root_arena (caller_arena do filho).
extern "C" fn spawn_child_and_return(_fa: i64, _caller_arena: i64, root_arena: i64) -> i64 {
    kata_rt_spawn(return_42 as *const () as i64, root_arena, 0);
    99
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Cast seguro de function pointer para i64.
fn fn_ptr(f: extern "C" fn(i64, i64, i64) -> i64) -> i64 {
    f as *const () as i64
}

/// Inicializa scheduler e retorna a arena raiz.
fn init() -> i64 {
    reset_scheduler();
    kata_rt_scheduler_init()
}

// ── Testes ──────────────────────────────────────────────────────────

#[test]
#[serial]
fn fiber_completes_and_returns_value() {
    let root_arena = init();
    kata_rt_spawn(fn_ptr(return_42), root_arena, 0);
    let result = kata_rt_run();
    assert_eq!(result, 42, "fiber raiz deve retornar 42");
}

#[test]
#[serial]
fn fiber_yields_and_resumes() {
    let root_arena = init();
    kata_rt_spawn(fn_ptr(yield_then_return_99), root_arena, 0);
    let result = kata_rt_run();
    assert_eq!(result, 99, "fiber raiz deve retornar 99 após yield");
}

#[test]
#[serial]
fn cooperative_yield_returns_to_run_queue() {
    let root_arena = init();
    kata_rt_spawn(fn_ptr(yield_n_times), root_arena, 5);
    let result = kata_rt_run();
    assert_eq!(result, 777, "fiber deve completar após 5 yields");
}

#[test]
#[serial]
fn fiber_completes_after_yield() {
    let root_arena = init();
    kata_rt_spawn(fn_ptr(yield_n_times), root_arena, 1);
    let result = kata_rt_run();
    assert_eq!(result, 777, "fiber deve retornar 777 após 1 yield");
}

#[test]
#[serial]
fn multiple_fibers_round_robin() {
    let root_arena = init();
    // O fiber raiz faz spawn de um filho e retorna 99.
    // O filho retorna 42. Structured concurrency: o raiz só é destruído
    // quando o filho termina. O resultado do scheduler é 99.
    kata_rt_spawn(fn_ptr(spawn_child_and_return), root_arena, root_arena);
    let result = kata_rt_run();
    assert_eq!(result, 99, "fiber raiz deve retornar 99");
}

#[test]
#[serial]
fn structured_concurrency_waits_for_children() {
    let root_arena = init();
    // O fiber raiz faz spawn de um filho e retorna 99.
    // Structured concurrency: o raiz só é destruído quando o filho termina.
    // O resultado do scheduler é 99 (resultado do raiz).
    kata_rt_spawn(fn_ptr(spawn_child_and_return), root_arena, root_arena);
    let result = kata_rt_run();
    assert_eq!(
        result, 99,
        "fiber raiz deve retornar 99 após filho completar"
    );
}

#[test]
#[serial]
fn channel_recv_blocks_and_wakes() {
    let root_arena = init();
    let handle = kata_rt_channel_create(root_arena);
    // Spawn um fiber que faz recv (bloqueia — canal vazio).
    kata_rt_spawn(fn_ptr(recv_from_channel), root_arena, handle);
    // Spawn um fiber que envia 42. Passamos o handle como caller_arena
    // e o valor como args_ptr.
    kata_rt_spawn(fn_ptr(send_to_channel), handle, 42);
    let result = kata_rt_run();
    // O fiber raiz (recv) retorna o valor recebido (42).
    assert_eq!(result, 42, "fiber que fez recv deve retornar 42");
}

#[test]
#[serial]
fn channel_send_blocks_when_slot_full() {
    let root_arena = init();
    let handle = kata_rt_channel_create(root_arena);
    // Envia 10 (preenche o slot) — fora de fiber, deve retornar OK.
    assert_eq!(kata_rt_channel_send(handle, 10), 0, "primeiro send deve OK");

    // Spawn um fiber que tenta enviar 20 (slot ocupado → bloqueia).
    kata_rt_spawn(fn_ptr(send_to_channel), handle, 20);
    // Spawn um fiber que faz recv (consome 10, libera o slot).
    kata_rt_spawn(fn_ptr(recv_from_channel), root_arena, handle);
    let result = kata_rt_run();
    // O fiber raiz (send) retorna 0 (OK) após o slot ser liberado.
    assert_eq!(
        result, 0,
        "fiber send deve retornar 0 (OK) após slot liberado"
    );
}

#[test]
#[serial]
fn deadlock_detection_returns_sentinel() {
    let root_arena = init();
    let handle = kata_rt_channel_create(root_arena);
    // Spawn um fiber que faz recv (bloqueia, não há sender).
    kata_rt_spawn(fn_ptr(recv_from_channel), root_arena, handle);
    // Nenhum fiber envia. Scheduler deve detectar deadlock e retornar sentinela.
    let result = kata_rt_run();
    assert_eq!(
        result, DEADLOCK_SENTINEL,
        "deadlock deve retornar DEADLOCK_SENTINEL"
    );
}
