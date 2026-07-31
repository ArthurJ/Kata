//! IPC — fork para `spawn!`.
//!
//! `spawn!` cria um processo OS separado via `fork()`. O child herda a
//! arena do parent via copy-on-write (COW), executa a Action, e termina.
//! Fire-and-forget — não há pipe de resultado, não há return.
//! A comunicação entre parent e child é exclusivamente por canais
//! (passados como args da Action).
//!
//! Esta module implementa a FFI `kata_rt_spawn_process` que o codegen
//! chama para realizar o fork.
//!
//! **Runtime completo no child:** após `fork()`, o child resetas as TLS
//! de runtime do parent (scheduler, suspend, yield) e bootstrapa um
//! scheduler próprio. Isto permite que o child execute Actions como fibers
//! (com cooperação, yield, select) em vez de chamadas diretas de função.
//! O child preserva os dados herdados via COW (arenas, type table, código JIT).

use crate::fiber::clear_suspend_tls;
use crate::scheduler::{reset_scheduler_tls, reset_yield_tls};

/// Spawn um processo OS separado para executar uma Action.
///
/// Fluxo:
/// 1. `fork()`
/// 2. **Child:** reseta TLS de runtime → inicializa scheduler próprio →
///    spawn da Action como fiber → `kata_rt_run()` → `_exit(0)`.
/// 3. **Parent:** retorna imediatamente (fire-and-forget).
///
/// O child herda todo o address space do parent via COW: código JIT, arenas,
/// FFIs registradas, type table. Apenas as TLS de runtime são resetadas para
/// que o child tenha um scheduler limpo — o parent estava em mid-execução
/// quando o fork ocorreu (dentro de `kata_rt_run` → `resume_fiber`), então as
/// TLS do parent apontam para scheduler/suspend em estado inválido para o child.
///
/// # Safety
/// - `fn_ptr` deve ser um ponteiro válido para `extern "C" fn(i64, i64, i64) -> i64`
///   (a Action JIT'd com ABI estendido — primeiro param é fiber_arena).
/// - `args_ptr` deve ser um ponteiro válido na arena do parent.
/// - `arena_handle` deve ser um handle de arena válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_spawn_process(fn_ptr: i64, args_ptr: i64, arena_handle: i64) -> i64 {
    // 1. fork()
    let pid = unsafe { libc::fork() };

    match pid {
        -1 => {
            // Erro — não foi possível forkar.
            0
        }
        0 => {
            // ── CHILD ──────────────────────────────────────
            // O child herda o address space do parent via COW, mas as TLS de
            // runtime apontam para o scheduler/suspend do parent (em mid-execução).
            // Resetar as TLS de runtime para ter um scheduler limpo.
            //
            // Ordem: clear_suspend_tls (dangling Suspend ptrs) →
            //        reset_scheduler_tls (SCHEDULER + PENDING_SPAWNS) →
            //        reset_yield_tls (YIELD_COUNTER + HAS_READY_FIBER).
            //
            // NÃO resetar: arenas (child precisa dos args via COW), type table
            // (child precisa para marshalling), snapshots, log config.
            clear_suspend_tls();
            reset_scheduler_tls();
            reset_yield_tls();

            // Inicializar scheduler próprio do child. Cria uma nova root arena.
            // A arena herdada do parent (arena_handle) é usada como caller_arena
            // — o child lê os args dela via COW.
            let child_root_arena = crate::scheduler::kata_rt_scheduler_init();

            // Spawn da Action como fiber. Usa a arena herdada como caller_arena
            // (args estão lá via COW) e a nova root arena do child como fiber_arena.
            // kata_rt_spawn registra no scheduler diretamente (is_in_fiber() == false
            // após clear_suspend_tls).
            crate::scheduler::kata_rt_spawn(fn_ptr, arena_handle, args_ptr);

            // Executar o scheduler até todos os fibers completarem.
            let _ = crate::scheduler::kata_rt_run();

            // Child termina. Não há pipe, não há resultado para enviar.
            // Suprimir warning de unused — child_root_arena é valida durante o run.
            let _ = child_root_arena;
            unsafe {
                libc::_exit(0);
            }
        }
        _pid => {
            // ── PARENT ─────────────────────────────────────
            // Fire-and-forget — não espera o child, não lê pipe.
            // Reap zombie assincronamente (SIGCHLD ou waitpid posterior).
            // Por ora, não reap aqui — o processo child é short-lived
            // e o parent pode continuar executando.
            // TODO: instalar handler SIGCHLD para reap automático.

            // Ignorar SIGPIPE no parent: se o child morrer antes de ler
            // todos os dados do pipe, um write subsequente gera SIGPIPE e
            // mataria o parent. SIG_IGN faz o write retornar EPIPE em vez
            // de matar o processo — o runtime trata EPIPE como erro
            // recuperável (WOULD_BLOCK ou valor sentinela).
            unsafe {
                libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            }
            0
        }
    }
}
