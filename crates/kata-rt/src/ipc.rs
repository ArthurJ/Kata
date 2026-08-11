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

use std::sync::Once;

use crate::fiber::clear_suspend_tls;
use crate::runtime::Runtime;

/// Instala `SIG_IGN` para SIGCHLD e SIGPIPE uma única vez, antes do primeiro
/// `fork()`.
///
/// **SIGCHLD:** o kernel descarta o status do child automaticamente — nenhum
/// zombie é criado. Sem handler, sem thread, sem loop de `waitpid`. Compatível
/// com o scheduler single-threaded (o kernel descarta sem invocar handler,
/// eliminando risco de reentrância).
///
/// `spawn!` é fire-and-forget por design — toda comunicação entre actions é
/// por canais, nunca por join/waitpid. Portanto `SIG_IGN` é a solução
/// definitiva, não provisória.
///
/// **SIGPIPE:** write em pipe cujo reader morreu retorna `EPIPE` em vez de
/// matar o processo. Instalado antes do `fork()` para que o child herde
/// ambos os dispositions — o child faz IPC via pipes e precisa do mesmo
/// comportamento.
///
/// `SA_RESTART` evita `EINTR` em syscalls bloqueantes do parent.
static INSTALL_SIGNAL_HANDLERS: Once = Once::new();

fn ensure_signal_handlers() {
    INSTALL_SIGNAL_HANDLERS.call_once(|| unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_IGN;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigaction(libc::SIGCHLD, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGPIPE, &sa, std::ptr::null_mut());
    });
}

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
pub extern "C" fn kata_rt_spawn_process(
    _rt: i64,
    fn_ptr: i64,
    args_ptr: i64,
    arena_handle: i64,
) -> i64 {
    // Instalar SIG_IGN para SIGCHLD e SIGPIPE antes do fork(). Idempotente
    // via Once — programas que nunca fazem spawn! não são afetados.
    // O child herda ambos os dispositions via fork().
    ensure_signal_handlers();

    // 1. fork()
    let pid = unsafe { libc::fork() };

    match pid {
        -1 => {
            // Erro — não foi possível forkar.
            0
        }
        0 => {
            // ── CHILD ──────────────────────────────────────
            // O child herda o address space do parent via COW, mas o
            // Runtime do parent (scheduler, arenas) está em mid-execução.
            // Limpar TLS de Suspend (dangling) e criar um novo Runtime.
            clear_suspend_tls();

            // Alocar um novo Runtime para o child.
            let child_rt = Box::new(Runtime::new());
            let rt_ptr = Box::into_raw(child_rt) as i64;

            // Spawn da Action como fiber. Usa a arena herdada como caller_arena
            // (args estão lá via COW) e a nova root arena do child como fiber_arena.
            crate::scheduler::kata_rt_spawn(rt_ptr, fn_ptr, arena_handle, args_ptr);

            // Executar o scheduler até todos os fibers completarem.
            let _ = crate::scheduler::kata_rt_run(rt_ptr);

            // Child termina.
            unsafe {
                libc::_exit(0);
            }
        }
        _pid => {
            // ── PARENT ─────────────────────────────────────
            // Fire-and-forget — não espera o child, não lê pipe.
            // SIGCHLD e SIGPIPE já estão em SIG_IGN (instalados antes do
            // fork via ensure_signal_handlers). O kernel descarta o status
            // do child automaticamente — nenhum zombie acumula.
            0
        }
    }
}
