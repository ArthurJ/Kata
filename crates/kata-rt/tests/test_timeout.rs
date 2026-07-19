//! Testes do timeout cooperativo de teste (Decisão A — Fio 14 Fase 4).
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

use kata_rt::{
    DEADLOCK_SENTINEL, TIMEOUT_SENTINEL, kata_rt_run, kata_rt_scheduler_init,
    kata_rt_set_test_timeout, kata_rt_spawn, kata_rt_yield_check, reset_scheduler,
};
use serial_test::serial;

/// Cast seguro de function pointer para i64 (pitfall #40 — Rust 2024).
fn fn_ptr(f: extern "C" fn(i64, i64, i64) -> i64) -> i64 {
    f as *const () as i64
}

/// Inicializa scheduler e retorna a arena raiz.
fn init() -> i64 {
    reset_scheduler();
    kata_rt_scheduler_init()
}

/// Loop infinito cooperativo — chama `kata_rt_yield_check` a cada iteração.
/// O codegen injeta `kata_rt_yield_check` no header de `Loop`/`ForIn`; este
/// teste simula esse comportamento chamando a FFI diretamente.
extern "C" fn infinite_loop(_fa: i64, _ca: i64, _args: i64) -> i64 {
    loop {
        kata_rt_yield_check();
    }
}

/// Fiber que termina rapidamente (retorna 42) — para testar cancelamento
/// do timer sem falso positivo.
extern "C" fn return_42(_fa: i64, _ca: i64, _args: i64) -> i64 {
    42
}

#[test]
#[serial]
fn timeout_expires_returns_timeout_sentinel() {
    let root_arena = init();
    kata_rt_set_test_timeout(100); // 100ms
    kata_rt_spawn(fn_ptr(infinite_loop), root_arena, 0);
    let result = kata_rt_run();
    assert_eq!(
        result, TIMEOUT_SENTINEL,
        "fiber em loop infinito com timeout de 100ms deve retornar TIMEOUT_SENTINEL"
    );
    assert_ne!(
        result, DEADLOCK_SENTINEL,
        "timeout não deve ser confundido com deadlock"
    );
}

#[test]
#[serial]
fn fiber_completes_before_timeout_returns_normal() {
    let root_arena = init();
    // Timeout longo (10s) — fiber termina em ~0ms.
    kata_rt_set_test_timeout(10_000);
    kata_rt_spawn(fn_ptr(return_42), root_arena, 0);
    let result = kata_rt_run();
    assert_eq!(
        result, 42,
        "fiber que termina antes do deadline deve retornar resultado normal"
    );
    assert_ne!(
        result, TIMEOUT_SENTINEL,
        "não deve retornar TIMEOUT_SENTINEL quando fiber completa antes do deadline"
    );
}

#[test]
#[serial]
fn reset_scheduler_clears_timeout_flag_no_false_positive() {
    // Primeiro teste: timeout dispara.
    let root_arena = init();
    kata_rt_set_test_timeout(100);
    kata_rt_spawn(fn_ptr(infinite_loop), root_arena, 0);
    let r1 = kata_rt_run();
    assert_eq!(r1, TIMEOUT_SENTINEL, "primeiro teste deve dar timeout");

    // Segundo teste: SEM timeout. Se reset_scheduler não limpasse
    // TIMEOUT_EXPIRED, o segundo teste falharia com falso positivo
    // (fiber retornaria TIMEOUT_SENTINEL mesmo sem timer configurado).
    let root_arena = init();
    // NÃO chama kata_rt_set_test_timeout — sem timer.
    kata_rt_spawn(fn_ptr(return_42), root_arena, 0);
    let r2 = kata_rt_run();
    assert_eq!(
        r2, 42,
        "segundo teste sem timeout deve retornar resultado normal — \
         reset_scheduler limpou TIMEOUT_EXPIRED"
    );
}

#[test]
#[serial]
fn timer_cancellation_does_not_set_flag() {
    // Configura timeout longo, deixa fiber terminar antes, verifica que
    // a thread cancelada NÃO setou TIMEOUT_EXPIRED. O segundo run (sem
    // timeout) deve retornar resultado normal, provando que não há falso
    // positivo.
    let root_arena = init();
    kata_rt_set_test_timeout(5_000); // 5s — fiber termina em ~0ms.
    kata_rt_spawn(fn_ptr(return_42), root_arena, 0);
    let r1 = kata_rt_run();
    assert_eq!(r1, 42, "fiber deve terminar antes do timeout");

    // reset_scheduler faz unpark + join na thread timer. A thread acordou
    // por unpark (não por timeout), Instant::now() < deadline, não seta
    // a flag. O segundo run (sem timeout) deve retornar normal.
    let root_arena = init();
    kata_rt_spawn(fn_ptr(return_42), root_arena, 0);
    let r2 = kata_rt_run();
    assert_eq!(
        r2, 42,
        "thread cancelada não deve setar TIMEOUT_EXPIRED — \
         distinção TimedOut vs Unparked via Instant::now() >= deadline"
    );
}