//! Fiber — wrapper sobre wasmtime-fiber para execução de Actions.
//!
//! Cada fiber tem sua própria stack (1 MB) e arena local. O scheduler cria
//! o fiber via `spawn`, passa `SpawnArgs` no `resume()`, e o trampoline
//! chama a função JIT com a assinatura uniforme `(fiber_arena, caller_arena, args_ptr) -> i64`.
//!
//! `Fiber<'static, SpawnArgs, YieldReason, i64>`:
//! - `Resume = SpawnArgs` — argumentos passados do host para o fiber
//! - `Yield = YieldReason` — razão pela qual o fiber suspendeu
//! - `Return = i64` — resultado da função JIT
//!
//! **TLS Suspend:** o trampoline guarda `&mut Suspend` como raw pointer em
//! `CURRENT_SUSPEND`. As FFIs (`kata_rt_yield`, `kata_rt_channel_recv` com
//! blocking) recuperam o ponteiro e chamam `suspend(YieldReason)`.
//! Single-threaded — sem data race. O TLS é limpo ao sair do trampoline.

use std::cell::Cell;
use wasmtime_fiber::{Fiber, FiberStack};

/// Tamanho da stack de cada fiber.
const FIBER_STACK_SIZE: usize = 1024 * 1024; // 1 MB

/// Ponteiro para o `Suspend` de um fiber. Armazenado no `KataFiber` para
/// que o scheduler re-sete `CURRENT_SUSPEND` antes de cada `resume()`.
///
/// O `Suspend` é criado no stack do fiber em `fiber_start` (wasmtime-fiber)
/// e reusado em todas as suspensões/resumes do mesmo fiber — o endereço
/// é estável durante todo o ciclo de vida do fiber.
pub(crate) type SuspendPtr = *mut wasmtime_fiber::Suspend<SpawnArgs, YieldReason, i64>;

/// Argumentos passados do scheduler para o fiber via `resume()`.
///
/// `#[repr(C)]` garante layout determinístico para a fronteira FFI.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SpawnArgs {
    /// Ponteiro da função JIT a executar (transmutado para `extern "C" fn`).
    pub(crate) fn_ptr: i64,
    /// Ponteiro do Runtime (`*mut Runtime` como i64). A2: passado pelo
    /// scheduler para que o fiber tenha acesso ao runtime nas FFIs centrais.
    pub(crate) rt: i64,
    /// Arena do caller — onde alocar valores de retorno (sobrevivem ao fiber).
    pub(crate) caller_arena: i64,
    /// Ponteiro para a tupla de argumentos (0 se Unit).
    pub(crate) args_ptr: i64,
    /// Arena do fiber — criada pelo scheduler, destruída após o fiber retornar.
    pub(crate) fiber_arena: i64,
}

/// Razão pela qual um fiber suspendeu execução (yield).
///
/// O scheduler interpreta o `YieldReason` retornado por `resume()` e decide
/// o que fazer: `Cooperative` → volta para run_queue; channel/select →
/// vai para blocked.
#[derive(Debug, Clone)]
pub(crate) enum YieldReason {
    /// Fiber fez yield cooperativo (back-edge check). Volta para run_queue.
    Cooperative,
    /// Fiber bloqueou esperando dado em canal (recv). `i64` = handle do canal.
    WaitingOnChannel(i64),
    /// Fiber bloqueou esperando espaço em canal (send). `i64` = handle do canal.
    WaitingOnChannelSend(i64),
    /// Fiber bloqueou em select. Carrega handles de canal e file separados.
    /// `deadline` de timeout (None = sem timeout, espera indefinidamente).
    WaitingOnSelect {
        channel_handles: Vec<i64>,
        file_handles: Vec<i64>,
        socket_handles: Vec<i64>,
        deadline: Option<std::time::Instant>,
    },
    /// Fiber fez sleep cooperativo. `Instant` = deadline para acordar.
    Sleep(std::time::Instant),
    /// Timeout de teste expirado (`@test(timeout: N)`). O fiber estava em
    /// loop/cooperative yield e a thread OS timer setou `TIMEOUT_EXPIRED`.
    /// O scheduler drena os fibers (sem dropar — pitfall #43) e retorna
    /// `Err("timeout")`, que `kata_rt_run` mapeia para `TIMEOUT_SENTINEL`.
    Timeout,
    /// Não usado — fiber completou. Existe para exaustividade do enum.
    #[allow(dead_code)] // variant usado em match arms mas não construído
    Done,
}

// TLS: ponteiro bruto para o `Suspend` do fiber atualmente em execução.
//
// `None` quando não há fiber em execução (ex: teste unitário chamando FFI
// diretamente). As FFIs de yield verificam: se `None`, operam em modo
// non-blocking (retornam WOULD_BLOCK); se `Some`, suspendem o fiber.
//
// Single-threaded — o ponteiro é válido apenas durante `resume()`.
// O trampoline limpa o TLS ao sair (set to None) para que chamadas
// FFI fora de fibers não suspendam nada.
thread_local! {
    static CURRENT_SUSPEND: Cell<*mut wasmtime_fiber::Suspend<SpawnArgs, YieldReason, i64>> =
        const { Cell::new(std::ptr::null_mut()) };

    // TLS onde o trampoline publica o `Suspend` do fiber que está executando.
    // O scheduler lê esta TLS após cada `resume()` que retorna `Err(YieldReason)`
    // (fiber suspendeu) para capturar o `Suspend` ptr daquele fiber — assim,
    // resumes subsequentes podem re-setar `CURRENT_SUSPEND` com o ptr correto.
    //
    // Necessário porque o `SuspendGuard` assume disciplina de pilha
    // (save/restore nested), mas fibers suspendem/retomam em ordem arbitrária:
    // quando A suspende e B resumiu entre-temps, `CURRENT_SUSPEND` ainda contém
    // o `Suspend` de B. O scheduler re-seta `CURRENT_SUSPEND` antes de cada
    // `resume()` com o `Suspend` capturado aqui.
    static LAST_SUSPEND_PTR: Cell<*mut wasmtime_fiber::Suspend<SpawnArgs, YieldReason, i64>> =
        const { Cell::new(std::ptr::null_mut()) };
}

/// Guarda o ponteiro do `Suspend` no TLS. Retornado pelo trampoline para
/// limpar o TLS após a execução.
struct SuspendGuard {
    old_ptr: *mut wasmtime_fiber::Suspend<SpawnArgs, YieldReason, i64>,
}

impl SuspendGuard {
    fn new(suspend: &mut wasmtime_fiber::Suspend<SpawnArgs, YieldReason, i64>) -> Self {
        let ptr: *mut _ = suspend;
        let old_ptr = CURRENT_SUSPEND.get();
        CURRENT_SUSPEND.set(ptr);
        SuspendGuard { old_ptr }
    }
}

impl Drop for SuspendGuard {
    fn drop(&mut self) {
        CURRENT_SUSPEND.set(self.old_ptr);
    }
}

/// Trampoline: recebe `SpawnArgs` e chama a função JIT.
///
/// Guarda o `Suspend` no TLS para que `kata_rt_yield()` e FFIs de canal
/// possam acessá-lo. O `SuspendGuard` limpa o TLS ao sair (mesmo se a
/// função JIT paniquar).
///
/// **Publicação do `Suspend` para resumes futuros:** o `SuspendGuard`
/// assume disciplina de pilha (save/restore nested), mas fibers suspendem/
/// retomam em ordem arbitrária. Quando A suspende e B resumiu entre-temps,
/// `CURRENT_SUSPEND` contém o `Suspend` de B ao resumir A. O scheduler
/// re-seta `CURRENT_SUSPEND` antes de cada `resume()` com o `Suspend` ptr
/// publicado aqui (estável por todo o ciclo de vida do fiber — criado em
/// `fiber_start` no stack do fiber).
fn trampoline(
    args: SpawnArgs,
    suspend: &mut wasmtime_fiber::Suspend<SpawnArgs, YieldReason, i64>,
) -> i64 {
    let _guard = SuspendGuard::new(suspend);
    // Publica o `Suspend` ptr para o scheduler capturar após `resume()`.
    // O ptr é estável enquanto o fiber não completa — criado em `fiber_start`
    // no stack do fiber, reusado em todas as suspensões/resumes do mesmo fiber.
    LAST_SUSPEND_PTR.with(|cell| cell.set(suspend as *mut _));
    let func: extern "C" fn(i64, i64, i64, i64) -> i64 =
        unsafe { core::mem::transmute(args.fn_ptr) };
    func(args.rt, args.fiber_arena, args.caller_arena, args.args_ptr)
}

/// Verifica se há um fiber em execução (Suspend em TLS).
///
/// Usado por `kata_rt_spawn` para decidir entre enfileirar o spawn
/// (dentro de fiber) ou acessar o scheduler diretamente (fora de fiber).
pub(crate) fn is_in_fiber() -> bool {
    CURRENT_SUSPEND.with(|cell| !cell.get().is_null())
}

/// Limpa as TLS de Suspend (`CURRENT_SUSPEND` e `LAST_SUSPEND_PTR`).
///
/// Chamado por `reset_scheduler` entre testes. Após timeout/drain (fibers
/// esquecidos sem dropar — pitfall #43), `CURRENT_SUSPEND` pode apontar
/// para o `Suspend` de um fiber drenado (dangling). Sem esta limpeza,
/// `is_in_fiber()` retorna `true` no próximo teste e `kata_rt_spawn`
/// enfileira em `PENDING_SPAWNS` em vez de registrar no scheduler — o fiber
/// nunca é criado e `root_result` fica `0`.
pub(crate) fn clear_suspend_tls() {
    CURRENT_SUSPEND.with(|cell| cell.set(std::ptr::null_mut()));
    LAST_SUSPEND_PTR.with(|cell| cell.set(std::ptr::null_mut()));
}

/// Re-seta `CURRENT_SUSPEND` com o `Suspend` de um fiber que será resumido.
///
/// O `SuspendGuard` no trampoline assume disciplina de pilha (save/restore
/// nested), mas fibers suspendem/retomam em ordem arbitrária. Quando A
/// suspende e B resumiu entre-temps, `CURRENT_SUSPEND` contém o `Suspend`
/// de B ao resumir A — A usaria o `Suspend` errado e panicaria em
/// `wasmtime-fiber/nostd.rs:188` (`assert!(!ret.is_null())`).
///
/// O scheduler chama esta função antes de cada `resume()` com o `Suspend`
/// capturado via `take_last_suspend()` após o `resume()` anterior.
pub(crate) fn set_current_suspend(ptr: SuspendPtr) {
    CURRENT_SUSPEND.with(|cell| cell.set(ptr));
}

/// Consome o `Suspend` ptr publicado pelo trampoline na última execução
/// de um fiber. O scheduler chama após cada `resume()` que retorna
/// `Err(YieldReason)` (fiber suspendeu) para armazenar o `Suspend` ptr
/// daquele fiber e re-setar `CURRENT_SUSPEND` no próximo `resume()`.
///
/// Retorna `None` se o trampoline não publicou (ex: fiber completou sem
/// suspender — `resume()` retornou `Ok`).
pub(crate) fn take_last_suspend() -> Option<SuspendPtr> {
    LAST_SUSPEND_PTR.with(|cell| {
        let p = cell.get();
        cell.set(std::ptr::null_mut());
        if p.is_null() { None } else { Some(p) }
    })
}

/// Tenta obter uma referência mutável ao `Suspend` do fiber atual.
///
/// Retorna `None` se não há fiber em execução (fora de `resume()`).
/// Seguro em single-threaded: o ponteiro só é válido durante `resume()`.
pub(crate) fn with_suspend<R>(
    f: impl FnOnce(&mut wasmtime_fiber::Suspend<SpawnArgs, YieldReason, i64>) -> R,
) -> Option<R> {
    CURRENT_SUSPEND.with(|cell| {
        let ptr = cell.get();
        if ptr.is_null() {
            None
        } else {
            // SAFETY: ptr é válido apenas durante resume() — single-threaded.
            // O trampoline garante que o ponteiro aponta para o Suspend correto.
            Some(f(unsafe { &mut *ptr }))
        }
    })
}

/// Um fiber Kata com sua arena associada.
pub(crate) struct KataFiber {
    fiber: Fiber<'static, SpawnArgs, YieldReason, i64>,
    /// Handle da arena do fiber — destruída quando o fiber termina.
    pub(crate) arena_handle: i64,
}

impl KataFiber {
    /// Cria um novo fiber com stack de 1 MB e arena associada.
    ///
    /// O caller é responsável por criar a arena (`kata_rt_arena_create`) e
    /// passar o handle aqui. A arena é destruída pelo scheduler após `resume`.
    pub(crate) fn new(arena_handle: i64) -> Result<Self, String> {
        let stack = FiberStack::new(FIBER_STACK_SIZE, false)
            .map_err(|e| format!("failed to create fiber stack: {e}"))?;
        let fiber =
            Fiber::new(stack, trampoline).map_err(|e| format!("failed to create fiber: {e}"))?;
        Ok(KataFiber {
            fiber,
            arena_handle,
        })
    }

    /// Resume o fiber com os argumentos fornecidos.
    ///
    /// Retorna `Ok(i64)` se o fiber completou, `Err(YieldReason)` se suspendeu.
    pub(crate) fn resume(&self, args: SpawnArgs) -> Result<i64, YieldReason> {
        self.fiber.resume(args)
    }
}
