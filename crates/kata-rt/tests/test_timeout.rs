//! Testes do timeout cooperativo de teste.
//!
//! Valida que:
//! 1. `kata_rt_set_test_timeout(N)` + fiber em loop infinito cooperativo
//!    → `kata_rt_run` retorna `TIMEOUT_SENTINEL` (não `DEADLOCK_SENTINEL`,
//!    não hang).
//! 2. `kata_rt_set_test_timeout(N)` + fiber que termina antes do deadline
//!    → `kata_rt_run` retorna resultado normal (não `TIMEOUT_SENTINEL`).
//!    Após `reset_scheduler`, a flag `TIMEOUT_EXPIRED` está limpa — a thread
//!    cancelada não seta a flag (distingue `TimedOut` de `Unparked` via
//!    comparação `Instant::now() >= deadline`), evitando falso positivo no
//!    próximo teste.
//!
//! Estes testes são `#[serial]` porque `TIMEOUT_EXPIRED` e `PENDING_TIMER`
//! são statics globais (não-TLS) — `kata_rt_run` não pode ser chamada de
//! múltiplas threads concorrentemente.
//!
//! A2: Funções de teste recebem `(rt, fiber_arena, caller_arena, args_ptr)`.

use kata_rt::{
    DEADLOCK_SENTINEL, Runtime, TIMEOUT_SENTINEL, kata_rt_run, kata_rt_scheduler_init,
    kata_rt_set_test_timeout, kata_rt_spawn, kata_rt_yield_check, reset_scheduler,
};
use serial_test::serial;

/// Cast seguro de function pointer para i64 (Rust 2024).
fn fn_ptr(f: extern "C" fn(i64, i64, i64, i64) -> i64) -> i64 {
    f as *const () as i64
}

/// A2: Aloca um Runtime fresco, inicializa scheduler. Retorna (rt_ptr, root_arena).
fn init() -> (i64, i64) {
    reset_scheduler();
    let rt = Box::new(Runtime::new());
    let rt_ptr = Box::into_raw(rt) as i64;
    let root_arena = kata_rt_scheduler_init(rt_ptr);
    (rt_ptr, root_arena)
}

/// A2: Descarta o Runtime após a execução.
fn cleanup(rt_ptr: i64) {
    unsafe { drop(Box::from_raw(rt_ptr as *mut Runtime)) };
}

/// Loop infinito cooperativo — chama `kata_rt_yield_check` a cada iteração.
/// O codegen injeta `kata_rt_yield_check(rt)` no header de `Loop`/`ForIn`.
extern "C" fn infinite_loop(rt: i64, _fa: i64, _ca: i64, _args: i64) -> i64 {
    loop {
        kata_rt_yield_check(rt);
    }
}

/// Fiber que termina rapidamente (retorna 42).
extern "C" fn return_42(_rt: i64, _fa: i64, _ca: i64, _args: i64) -> i64 {
    42
}

#[test]
#[serial]
fn timeout_expires_returns_timeout_sentinel() {
    let (rt, root_arena) = init();
    kata_rt_set_test_timeout(100); // 100ms
    kata_rt_spawn(rt, fn_ptr(infinite_loop), root_arena, 0);
    let result = kata_rt_run(rt);
    assert_eq!(
        result, TIMEOUT_SENTINEL,
        "fiber em loop infinito com timeout de 100ms deve retornar TIMEOUT_SENTINEL"
    );
    assert_ne!(
        result, DEADLOCK_SENTINEL,
        "timeout não deve ser confundido com deadlock"
    );
    cleanup(rt);
}

#[test]
#[serial]
fn fiber_completes_before_timeout_returns_normal() {
    let (rt, root_arena) = init();
    // Timeout longo (10s) — fiber termina em ~0ms.
    kata_rt_set_test_timeout(10_000);
    kata_rt_spawn(rt, fn_ptr(return_42), root_arena, 0);
    let result = kata_rt_run(rt);
    assert_eq!(
        result, 42,
        "fiber que termina antes do deadline deve retornar resultado normal"
    );
    assert_ne!(
        result, TIMEOUT_SENTINEL,
        "não deve retornar TIMEOUT_SENTINEL quando fiber completa antes do deadline"
    );
    cleanup(rt);
}

#[test]
#[serial]
fn reset_scheduler_clears_timeout_flag_no_false_positive() {
    // Primeiro teste: timeout dispara.
    let (rt, root_arena) = init();
    kata_rt_set_test_timeout(100);
    kata_rt_spawn(rt, fn_ptr(infinite_loop), root_arena, 0);
    let r1 = kata_rt_run(rt);
    assert_eq!(r1, TIMEOUT_SENTINEL, "primeiro teste deve dar timeout");
    cleanup(rt);

    // Segundo teste: SEM timeout. Se reset_scheduler não limpasse
    // TIMEOUT_EXPIRED, o segundo teste falharia com falso positivo
    // (fiber retornaria TIMEOUT_SENTINEL mesmo sem timer configurado).
    let (rt, root_arena) = init();
    // NÃO chama kata_rt_set_test_timeout — sem timer.
    kata_rt_spawn(rt, fn_ptr(return_42), root_arena, 0);
    let r2 = kata_rt_run(rt);
    assert_eq!(
        r2, 42,
        "segundo teste sem timeout deve retornar resultado normal — \
         reset_scheduler limpou TIMEOUT_EXPIRED"
    );
    cleanup(rt);
}

#[test]
#[serial]
fn timer_cancellation_does_not_set_flag() {
    // Configura timeout longo, deixa fiber terminar antes, verifica que
    // a thread cancelada NÃO setou TIMEOUT_EXPIRED. O segundo run (sem
    // timeout) deve retornar resultado normal, provando que não há falso
    // positivo.
    let (rt, root_arena) = init();
    kata_rt_set_test_timeout(5_000); // 5s — fiber termina em ~0ms.
    kata_rt_spawn(rt, fn_ptr(return_42), root_arena, 0);
    let r1 = kata_rt_run(rt);
    assert_eq!(r1, 42, "fiber deve terminar antes do timeout");
    cleanup(rt);

    // reset_scheduler faz unpark + join na thread timer. A thread acordou
    // por unpark (não por timeout), Instant::now() < deadline, não seta
    // a flag. O segundo run (sem timeout) deve retornar normal.
    let (rt, root_arena) = init();
    kata_rt_spawn(rt, fn_ptr(return_42), root_arena, 0);
    let r2 = kata_rt_run(rt);
    assert_eq!(
        r2, 42,
        "thread cancelada não deve setar TIMEOUT_EXPIRED — \
         distinção TimedOut vs Unparked via Instant::now() >= deadline"
    );
    cleanup(rt);
}
