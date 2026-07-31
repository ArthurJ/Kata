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

/// Spawn um processo OS separado para executar uma Action.
///
/// Fluxo:
/// 1. `fork()`
/// 2. **Child:** chama a Action com a arena herdada (COW), termina.
/// 3. **Parent:** retorna imediatamente (fire-and-forget).
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
            // O child herda a arena via COW. Chama a Action diretamente.
            // A Action tem ABI: (fiber_arena, caller_arena, args_ptr) -> i64.
            // O child usa a arena herdada como ambas (fiber e caller).
            let action: extern "C" fn(i64, i64, i64) -> i64 =
                unsafe { std::mem::transmute(fn_ptr) };
            let _ = action(arena_handle, arena_handle, args_ptr);

            // Child termina. Não há pipe, não há resultado para enviar.
            unsafe {
                libc::_exit(0);
            }
        }
        pid => {
            // ── PARENT ─────────────────────────────────────
            // Fire-and-forget — não espera o child, não lê pipe.
            // Reap zombie assincronamente (SIGCHLD ou waitpid posterior).
            // Por ora, não reap aqui — o processo child é short-lived
            // e o parent pode continuar executando.
            // TODO: instalar handler SIGCHLD para reap automático.
            0
        }
    }
}
