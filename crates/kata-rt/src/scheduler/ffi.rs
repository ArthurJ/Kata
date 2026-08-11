//! FFI layer do scheduler — funções C-ABI expostas ao codegen.
//!
//! O `Scheduler` struct + impl vivem no módulo pai (`scheduler.rs`); este
//! submódulo concentra as funções `extern "C"` chamadas pelo codegen JIT
//! (`kata_rt_scheduler_init`, `kata_rt_spawn`, `kata_rt_run`, `kata_rt_yield`,
//! `kata_rt_yield_check`), além de `DEADLOCK_SENTINEL` e `TIMEOUT_SENTINEL`.
//!
//! A2 — Runtime reentrante: o estado antes em TLS (`SCHEDULER`,
//! `PENDING_SPAWNS`, `YIELD_COUNTER`, `HAS_READY_FIBER`) agora vive na struct
//! `Runtime` (ver `runtime.rs`). As FFIs recebem `rt: i64` (ponteiro para
//! `*mut Runtime`) como primeiro parâmetro. O `PENDING_SPAWNS` workaround
//! foi eliminado — o scheduler é acessado via ponteiro direto, sem RefCell.
//!
//! `TIMEOUT_EXPIRED` e `PENDING_TIMER` permanecem globais (não-TLS) — a
//! thread OS timer não tem acesso ao `Runtime*`.

use crate::fiber::{YieldReason, is_in_fiber, with_suspend};
use crate::runtime::Runtime;

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

// Handle da thread OS timer pendente. `reset_test_timer` faz `unpark + join`
// para cancelar a thread anterior antes de resetar `TIMEOUT_EXPIRED` — ordem
// obrigatória: `join` antes de `store(false)` (evita que a thread sete `true`
// após o reset, poluindo o próximo teste com falso positivo).
static PENDING_TIMER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// Reseta apenas o timer de teste global. Chamado entre execuções de teste.
///
/// Cancela e espera a thread OS timer anterior terminar (`unpark` + `join`),
/// depois reseta `TIMEOUT_EXPIRED`. Não toca o `Runtime` — o caller descarta
/// o Runtime antigo e cria um novo.
pub(crate) fn reset_test_timer() {
    if let Some(handle) = PENDING_TIMER
        .lock()
        .expect("PENDING_TIMER não envenenado")
        .take()
    {
        handle.thread().unpark();
        let _ = handle.join();
    }
    TIMEOUT_EXPIRED.store(false, Relaxed);
}

/// Limpa TLS de Suspend e log entre execuções de teste.
///
/// Com o `Runtime` explícito, o scheduler e as arenas são descartados com o
/// `Runtime`. Mas `CURRENT_SUSPEND` (TLS) pode apontar para um Suspend
/// dangling após timeout/drain, e o log TLS pode poluir. Esta função limpa
/// apenas o que permanece em TLS.
pub(crate) fn reset_tls_between_runs() {
    crate::fiber::clear_suspend_tls();
    crate::log::reset_log();
    crate::snapshot::reset_snapshot_table();
}

/// Reseta o timer de teste global E as TLS periféricas (Suspend, log, snapshot).
///
/// Substitui o antigo `reset_scheduler` — o scheduler em si é descartado
/// junto com o `Runtime`. Esta função limpa apenas o que permanece em
/// TLS/global: timer de teste, Suspend TLS, log TLS, snapshot table.
pub fn reset_scheduler() {
    reset_test_timer();
    reset_tls_between_runs();
}

/// Inicializa o `Runtime` apontado por `rt` e retorna o handle da root arena.
///
/// O driver aloca um `Box<Runtime>` e passa o ponteiro bruto como `rt`.
/// Esta função inicializa o Runtime (que já foi criado por `Runtime::new()`
/// no driver) — na prática, é um no-op pois o `Runtime::new()` já fez tudo.
/// Mantida para compatibilidade de ABI com o codegen que espera chamá-la.
///
/// Retorna o handle da root arena para o codegen usar como `caller_arena`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_scheduler_init(rt: i64) -> i64 {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    // Cache do ponteiro em TLS para FFIs periféricas (array, list, dict, etc.)
    crate::arena::set_rt_ptr(rt);
    runtime.root_arena_handle
}

/// Cria um fiber com arena própria e o enfileira.
///
/// Chamado no `__kata_entry` quando encontra ActionCall definida pelo usuário.
///
/// Se chamado de dentro de um fiber (durante `resume()`), enfileira o
/// spawn em `scheduler.pending_spawns` — o scheduler drena a lista após
/// cada `resume()`. Isto é necessário porque durante `resume()` o
/// `scheduler.run()` tem `&mut self` ativo; o spawn não pode pegar
/// `&mut scheduler` simultaneamente.
///
/// Retorna o `FiberId` (i64). Quando enfileirado, retorna 0 (o FiberId
/// será atribuído quando o scheduler drenar).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_spawn(rt: i64, fn_ptr: i64, caller_arena: i64, args_ptr: i64) -> i64 {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    if is_in_fiber() {
        // Dentro de fiber — enfileirar em pending_spawns (campo do scheduler).
        runtime
            .scheduler
            .pending_spawns
            .push((fn_ptr, rt, caller_arena, args_ptr));
        0
    } else {
        // Fora de fiber — spawn direto.
        match runtime
            .scheduler
            .spawn(&mut runtime.arenas, fn_ptr, rt, caller_arena, args_ptr)
        {
            Ok(id) => id as i64,
            Err(_e) => {
                eprintln!("kata_rt_spawn: erro ao criar fiber: {_e}");
                0
            }
        }
    }
}

/// Executa o scheduler até todos os fibers completarem.
///
/// Retorna o resultado do fiber raiz (i64). Se deadlock for detectado,
/// imprime a mensagem no stderr e retorna `DEADLOCK_SENTINEL`. Se timeout
/// de teste expirar, retorna `TIMEOUT_SENTINEL` (distinto de `DEADLOCK_SENTINEL`).
///
/// Não faz `panic!` porque `extern \"C\"` é `nounwind` — um panic aqui
/// aborta com SIGABRT (non-unwinding). O chamador (driver/teste)
/// verifica o valor de retorno.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_run(rt: i64) -> i64 {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    let result = runtime.scheduler.run(&mut runtime.arenas);
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
/// A thread é cancelável: `reset_test_timer` faz `unpark + join` na thread
/// pendente antes de resetar `TIMEOUT_EXPIRED`. A thread OS só escreve no
/// `AtomicBool` isolado — não toca scheduler, arenas, nem Runtime.
///
/// `TIMEOUT_EXPIRED` é global (NÃO-TLS) — implica serialização de testes:
/// `kata_rt_run` não pode ser chamada de múltiplas threads concorrentemente.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_test_timeout(millis: i64) {
    // Cancelar thread timer anterior (se houver) antes de iniciar nova.
    if let Some(handle) = PENDING_TIMER
        .lock()
        .expect("PENDING_TIMER não envenenado")
        .take()
    {
        handle.thread().unpark();
        let _ = handle.join();
    }
    let deadline = Instant::now() + Duration::from_millis(millis as u64);
    let handle = thread::spawn(move || {
        thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
        if Instant::now() >= deadline {
            TIMEOUT_EXPIRED.store(true, Relaxed);
        }
    });
    *PENDING_TIMER.lock().expect("PENDING_TIMER não envenenado") = Some(handle);
}

/// Suspende o fiber atual com `YieldReason::Cooperative`.
///
/// Chamada pelo codegen em yield points ou por código Kata que
/// explicitamente cede CPU. Não acessa o scheduler — apenas suspende via
/// TLS `Suspend` (CURRENT_SUSPEND). O scheduler interpreta o `YieldReason`
/// quando `resume()` retorna `Err(YieldReason::Cooperative)` e coloca o fiber
/// de volta na run_queue.
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
/// Hot path (a cada iteração): decrementa `scheduler.yield_counter`. Se
/// ainda > 0, retorna imediatamente — 2 instruções no hot path (dec + branch).
///
/// Slow path (a cada `YIELD_INTERVAL` iterações): reseta o contador e checa
/// `scheduler.has_ready_fiber`. Se `true` (há outra fiber pronta na
/// run_queue), suspende cooperativamente. Se `false`, retorna sem suspender.
///
/// Recebe `rt` para acessar `yield_counter` e `has_ready_fiber` do Runtime.
///
/// Se chamada fora de um fiber (sem `Suspend` em TLS), é no-op em ambos os
/// caminhos — `with_suspend` não faz nada se não há `Suspend` ativo.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_yield_check(rt: i64) {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    runtime.scheduler.yield_counter -= 1;
    if runtime.scheduler.yield_counter > 0 {
        return;
    }
    runtime.scheduler.yield_counter = super::YIELD_INTERVAL;
    // Test timeout — sempre checa, ANTES do guard `has_ready_fiber`.
    if TIMEOUT_EXPIRED.load(Relaxed) {
        with_suspend(|s| s.suspend(YieldReason::Timeout));
        return;
    }
    if !runtime.scheduler.has_ready_fiber {
        return;
    }
    with_suspend(|suspend| {
        suspend.suspend(YieldReason::Cooperative);
    });
}

/// Sleep cooperativo — suspende o fiber atual até `ms` milissegundos decorridos.
///
/// O argumento `ms` é SMI-tagged (convenção do runtime). A FFI decodifica,
/// calcula o deadline, e suspende com `YieldReason::Sleep(deadline)`. O
/// scheduler coloca o fiber em `blocked` com `WaitingOnSleep(deadline)` e o
/// acorda quando `now >= deadline` (via `wake_pass` ou `earliest_deadline`).
///
/// Não recebe `rt` — só usa `with_suspend` (TLS), não acessa o scheduler.
///
/// Se chamada fora de um fiber (sem `Suspend` em TLS), é no-op.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_sleep(ms: i64) {
    // Decodificar SMI: (val << 1 | 1) >> 1 = val.
    let millis = ms >> 1;
    if millis <= 0 {
        return;
    }
    let deadline = Instant::now() + Duration::from_millis(millis as u64);
    with_suspend(|suspend| {
        suspend.suspend(YieldReason::Sleep(deadline));
    });
}
