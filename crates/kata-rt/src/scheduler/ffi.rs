//! FFI layer do scheduler — estado TLS e funções C-ABI expostas ao codegen.
//!
//! O `Scheduler` struct + impl vivem no módulo pai (`scheduler.rs`); este
//! submódulo concentra apenas a camada de integração FFI: o `thread_local!`
//! `SCHEDULER`, os contadores de yield (`YIELD_COUNTER`, `HAS_READY_FIBER`),
//! a fila de spawns pendentes (`PENDING_SPAWNS`) e as funções
//! `extern "C"` chamadas pelo codegen JIT (`kata_rt_scheduler_init`,
//! `kata_rt_spawn`, `kata_rt_run`, `kata_rt_yield`, `kata_rt_yield_check`),
//! além de `reset_scheduler` (usada entre testes) e `DEADLOCK_SENTINEL`.
//!
//! `PENDING_SPAWNS`, `YIELD_COUNTER`, `HAS_READY_FIBER` e `YIELD_INTERVAL`
//! são `pub(super)` porque o `Scheduler` impl no módulo pai os acessa
//! diretamente (em `resume_fiber` e `drain_pending_spawns`).

use crate::fiber::{YieldReason, is_in_fiber, with_suspend};

use super::Scheduler;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// ── Test timeout cooperativo ──────────────────────────
//
// `TIMEOUT_EXPIRED` é global (NÃO-TLS) porque precisa ser visível entre a
// thread OS timer e a thread do fiber. Setada pela thread timer quando o
// `park_timeout` expira; lida pelo `kata_rt_yield_check` slow path a cada
// `YIELD_INTERVAL` iterações. Implica serialização de testes — `kata_rt_run`
// não pode ser chamada de múltiplas threads concorrentemente (compatível com
// o scheduler single-threaded).
static TIMEOUT_EXPIRED: AtomicBool = AtomicBool::new(false);

// Handle da thread OS timer pendente. `reset_scheduler` faz `unpark + join`
// para cancelar a thread anterior antes de resetar `TIMEOUT_EXPIRED` — ordem
// obrigatória: `join` antes de `store(false)` (evita que a thread sete `true`
// após o reset, poluindo o próximo teste com falso positivo).
static PENDING_TIMER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

// ── Scheduler thread-local ────────────────────────────────────────────
// Scheduler thread-local — 1 por thread (single-threaded).
thread_local! {
    static SCHEDULER: std::cell::RefCell<Option<Scheduler>> =
        const { std::cell::RefCell::new(None) };
}

// Spawns pendentes — enfileirados por `kata_rt_spawn` quando chamado
// de dentro de um fiber (durante `resume()`). O scheduler drena esta
// lista após cada `resume()` no `run()`.
//
// Isto evita o double-borrow do SCHEDULER: durante `resume()`, o
// scheduler já tem o `RefCell` emprestado (`borrow_mut` em `run()`).
// Se a função JIT chamar `kata_rt_spawn`, um segundo `borrow_mut`
// causaria panic. Em vez disso, `kata_rt_spawn` detecta que está
// dentro de um fiber (Suspend em TLS) e enfileira o spawn aqui.
thread_local! {
    pub(super) static PENDING_SPAWNS: std::cell::RefCell<Vec<(i64, i64, i64)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

// ── Yield points ──────────────────────────────────────────────
//
// `kata_rt_yield_check()` é chamada pelo codegen no header de cada iteração
// de `Loop` e `ForIn`. Para evitar o custo de suspender a cada iteração,
// decrementa um contador TLS e só pergunta ao scheduler se há outra fiber
// pronta a cada YIELD_INTERVAL iterações.
//
// `HAS_READY_FIBER` é um snapshot booleano do estado do scheduler, setado
// antes de cada `resume()` em `resume_fiber`. Isto evita o double-borrow do
// `SCHEDULER` (pitfall #44): durante `resume()`, a função JIT pode chamar
// `kata_rt_yield_check`, que lê apenas esta TLS — sem acessar o RefCell.
pub(super) const YIELD_INTERVAL: i64 = 1000;

thread_local! {
    pub(super) static YIELD_COUNTER: std::cell::Cell<i64> =
        const { std::cell::Cell::new(YIELD_INTERVAL) };
    pub(super) static HAS_READY_FIBER: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// Reseta o scheduler thread-local. Chamado entre execuções de teste.
///
/// Primeiro cancela e espera a thread OS timer anterior terminar (`unpark` +
/// `join`) — sem isto, a thread anterior pode setar `TIMEOUT_EXPIRED = true`
/// após o reset e poluir o próximo teste com falso positivo. Depois reseta
/// `TIMEOUT_EXPIRED` (após `join`, a thread terminou — a flag está em estado
/// final). Por fim, reseta o resto (scheduler, spawns, contadores).
pub fn reset_scheduler() {
    // 1. Cancelar e esperar a thread timer anterior terminar.
    if let Some(handle) = PENDING_TIMER.lock().unwrap().take() {
        handle.thread().unpark();
        let _ = handle.join();
    }
    // 2. Resetar a flag (após join, a thread terminou — flag está em estado final).
    TIMEOUT_EXPIRED.store(false, Relaxed);
    // 3. Resetar o resto.
    SCHEDULER.with(|s| {
        s.borrow_mut().take();
    });
    PENDING_SPAWNS.with(|p| {
        p.borrow_mut().clear();
    });
    YIELD_COUNTER.with(|c| c.set(YIELD_INTERVAL));
    HAS_READY_FIBER.with(|h| h.set(false));
    // Limpar TLS de Suspend — após timeout/drain, CURRENT_SUSPEND pode
    // apontar para o Suspend de um fiber drenado (dangling). Sem isto,
    // is_in_fiber() retorna true no próximo teste e kata_rt_spawn
    // enfileira em PENDING_SPAWNS em vez de registrar no scheduler.
    crate::fiber::clear_suspend_tls();
}

/// Inicializa o scheduler thread-local e cria a arena raiz.
///
/// Retorna o handle da arena raiz para o codegen usar como `caller_arena`
/// no entry point.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_scheduler_init() -> i64 {
    SCHEDULER.with(|s| {
        let scheduler = Scheduler::new();
        let root_arena = scheduler.root_arena;
        *s.borrow_mut() = Some(scheduler);
        root_arena
    })
}

/// Cria um fiber com arena própria e o enfileira.
///
/// Chamado no `__kata_entry` quando encontra ActionCall definida pelo usuário.
///
/// Se chamado de dentro de um fiber (durante `resume()`), enfileira o
/// spawn em `PENDING_SPAWNS` em vez de acessar o SCHEDULER diretamente —
/// evita double-borrow do `RefCell`. O scheduler drena a lista após
/// cada `resume()`.
///
/// Retorna o `FiberId` (i64). Quando enfileirado, retorna 0 (o FiberId
/// será atribuído quando o scheduler drenar).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_spawn(fn_ptr: i64, caller_arena: i64, args_ptr: i64) -> i64 {
    if is_in_fiber() {
        // Dentro de fiber — enfileirar em PENDING_SPAWNS.
        PENDING_SPAWNS.with(|p| {
            p.borrow_mut().push((fn_ptr, caller_arena, args_ptr));
        });
        0
    } else {
        SCHEDULER.with(|s| {
            let mut s = s.borrow_mut();
            let scheduler = s.as_mut().expect("scheduler não inicializado");
            match scheduler.spawn(fn_ptr, caller_arena, args_ptr) {
                Ok(id) => id as i64,
                Err(_e) => {
                    eprintln!("kata_rt_spawn: erro ao criar fiber: {_e}");
                    0
                }
            }
        })
    }
}

/// Executa o scheduler até todos os fibers completarem.
///
/// Retorna o resultado do fiber raiz (i64). Se deadlock for detectado,
/// imprime a mensagem no stderr e retorna `DEADLOCK_SENTINEL`. Se timeout
/// de teste expirar, retorna `TIMEOUT_SENTINEL` (distinto de `DEADLOCK_SENTINEL`).
///
/// Não faz `panic!` porque `extern "C"` é `nounwind` — um panic aqui
/// aborta com SIGABRT (non-unwinding). O chamador (driver/teste)
/// verifica o valor de retorno.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_run() -> i64 {
    let result = SCHEDULER.with(|s| {
        let mut s = s.borrow_mut();
        let scheduler = s.as_mut().expect("scheduler não inicializado");
        scheduler.run()
    });
    match result {
        Ok(v) => v,
        Err(msg) if msg == "timeout" => TIMEOUT_SENTINEL,
        Err(msg) => {
            eprintln!("kata_rt_run: {msg}");
            DEADLOCK_SENTINEL
        }
    }
}

/// Valor retornado por `kata_rt_run` quando deadlock é detectado.
/// Permite que o chamador distinga deadlock de um resultado legítimo.
pub const DEADLOCK_SENTINEL: i64 = i64::MIN + 1;

/// Valor retornado por `kata_rt_run` quando timeout de teste expira
/// (`@test(timeout: N)`). Distinto de `DEADLOCK_SENTINEL` para que o
/// runner reporte "timeout" em vez de "deadlock".
pub const TIMEOUT_SENTINEL: i64 = i64::MIN + 2;

/// Configura o timeout de teste.
///
/// Spawna uma thread OS que faz `thread::park_timeout(Duration::from_millis(millis))`
/// direto (granularidade de ms aceitável para testes). Ao acordar, distingue:
/// - `ParkTimeoutResult::TimedOut` → seta `TIMEOUT_EXPIRED = true` (timeout real).
/// - `Unparked` → cancelada pelo runner (teste terminou antes), NÃO seta a flag
///   — evita falso positivo que poluiria o próximo teste.
///
/// A thread é cancelável: `reset_scheduler` faz `unpark + join` na thread
/// pendente antes de resetar `TIMEOUT_EXPIRED`. A thread OS só escreve no
/// `AtomicBool` isolado — não toca scheduler, arenas, nem TLS. A invariant
/// single-threaded é preservada.
///
/// `TIMEOUT_EXPIRED` é global (NÃO-TLS) — implica serialização de testes:
/// `kata_rt_run` não pode ser chamada de múltiplas threads concorrentemente.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_test_timeout(millis: i64) {
    // Cancelar thread timer anterior (se houver) antes de iniciar nova.
    if let Some(handle) = PENDING_TIMER.lock().unwrap().take() {
        handle.thread().unpark();
        let _ = handle.join();
    }
    let deadline = Instant::now() + Duration::from_millis(millis as u64);
    let handle = thread::spawn(move || {
        // `park_timeout` retorna `()` em Rust stable — não distingue timeout
        // de unpark. Para distinguir, comparamos `Instant::now()` com o
        // deadline após acordar. Se `now >= deadline`, foi timeout real;
        // senão, foi unpark (cancelada pelo runner) — não seta a flag.
        thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
        if Instant::now() >= deadline {
            TIMEOUT_EXPIRED.store(true, Relaxed);
        }
        // Unparked antes do deadline = cancelada — não seta a flag.
    });
    *PENDING_TIMER.lock().unwrap() = Some(handle);
}

/// Suspende o fiber atual com `YieldReason::Cooperative`.
///
/// Chamada pelo codegen em yield points ou por código Kata que
/// explicitamente cede CPU. Não acessa o scheduler — apenas suspende via
/// TLS `Suspend`. O scheduler interpreta o `YieldReason` quando `resume()`
/// retorna `Err(YieldReason::Cooperative)` e coloca o fiber de volta na
/// run_queue.
///
/// Se chamada fora de um fiber (sem `Suspend` em TLS), é no-op.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_yield() {
    with_suspend(|suspend| {
        suspend.suspend(YieldReason::Cooperative);
    });
}

/// Yield point chamado pelo codegen no header de cada iteração de `Loop` e
/// `ForIn` (Decisão G).
///
/// Hot path (a cada iteração): decrementa `YIELD_COUNTER` TLS. Se ainda > 0,
/// retorna imediatamente — 2 instruções no hot path (dec + branch).
///
/// Slow path (a cada `YIELD_INTERVAL` iterações): reseta o contador e checa
/// `HAS_READY_FIBER`. Se `true` (há outra fiber pronta na run_queue),
/// suspende cooperativamente via `kata_rt_yield`. Se `false`, retorna sem
/// suspender — evita suspend/resume desnecessário quando só há uma fiber.
///
/// `HAS_READY_FIBER` é setado pelo scheduler antes de cada `resume()` em
/// `resume_fiber`, evitando o double-borrow do `SCHEDULER` (pitfall #44):
/// esta FFI lê apenas a TLS, sem acessar o `RefCell` do scheduler.
///
/// Se chamada fora de um fiber (sem `Suspend` em TLS), é no-op em ambos os
/// caminhos — `with_suspend` não faz nada se não há `Suspend` ativo.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_yield_check() {
    let remaining = YIELD_COUNTER.with(|c| {
        let v = c.get() - 1;
        c.set(v);
        v
    });
    if remaining > 0 {
        return;
    }
    YIELD_COUNTER.with(|c| c.set(YIELD_INTERVAL));
    // Test timeout — sempre checa, ANTES do guard `HAS_READY_FIBER`. Runs de
    // teste com fiber único precisam detectar o timeout mesmo sem outra fiber
    // pronta. Runs normais (`TIMEOUT_EXPIRED = false`) não mudam comportamento.
    if TIMEOUT_EXPIRED.load(Relaxed) {
        with_suspend(|s| s.suspend(YieldReason::Timeout));
        return;
    }
    if !HAS_READY_FIBER.with(|h| h.get()) {
        return;
    }
    with_suspend(|suspend| {
        suspend.suspend(YieldReason::Cooperative);
    });
}
