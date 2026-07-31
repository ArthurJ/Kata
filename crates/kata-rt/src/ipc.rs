//! IPC — fork + pipe para `spawn!`.
//!
//! `spawn!` cria um processo OS separado via `fork()`. O child herda a
//! arena do parent via copy-on-write (COW), executa a Action, serializa
//! o resultado via `to_bytes`, e envia pelo pipe. O parent faz `yield`
//! (cede a outras fibers), lê o pipe, e desserializa via `from_bytes`.
//!
//! Esta module implementa a FFI `kata_rt_spawn_process` que o codegen
//! chama para realizar o fork+exec+pipe.

use crate::marshal::{kata_rt_from_bytes, kata_rt_to_bytes};
use std::io::{self, Read, Write};

/// Spawn sentinel — child escreve no pipe antes do resultado para
/// indicar que está pronto para enviar o payload.
const SPAWN_OK: u8 = 0;
const SPAWN_ERR: u8 = 1;

/// Tamanho do header do blob to_bytes (len + type_id + rebase_count).
const BLOB_HEADER_SIZE: usize = 24;

/// Spawn um processo OS separado para executar uma Action.
///
/// Fluxo:
/// 1. Cria pipe (parent→child para args, child→parent para result)
/// 2. `fork()`
/// 3. **Child:** lê args_ptr do pipe (herdado via COW, mas lê o
///    ponteiro do pipe), chama a Action, `to_bytes(result)`, escreve
///    no pipe, `exit(0)`
/// 4. **Parent:** `yield` (cede a outras fibers), lê o pipe,
///    `from_bytes(result)`, retorna o value_ptr
///
/// # Safety
/// - `fn_ptr` deve ser um ponteiro válido para `extern "C" fn(i64) -> i64`
///   (a Action JIT'd com ABI estendido — primeiro param é caller_arena).
/// - `args_ptr` deve ser um ponteiro válido na arena do parent.
/// - `result_type_id` deve ser um type_id válido na type table registrada.
/// - `arena_handle` deve ser um handle de arena válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_spawn_process(
    fn_ptr: i64,
    args_ptr: i64,
    result_type_id: i64,
    arena_handle: i64,
) -> i64 {
    spawn_process_inner(fn_ptr, args_ptr, result_type_id, arena_handle)
}

fn spawn_process_inner(
    fn_ptr: i64,
    args_ptr: i64,
    result_type_id: i64,
    arena_handle: i64,
) -> i64 {
    // 1. Criar pipe: child→parent para o resultado.
    let mut result_pipe: [i32; 2] = [0; 2];
    let rc = unsafe { libc::pipe(result_pipe.as_mut_ptr()) };
    if rc != 0 {
        return 0; // erro — não foi possível criar pipe
    }
    let read_fd = result_pipe[0];
    let write_fd = result_pipe[1];

    // 2. fork()
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => {
            // Erro — fecha pipes e retorna 0.
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            0
        }
        0 => {
            // ── CHILD ──────────────────────────────────────
            // Fecha o lado de leitura do pipe de resultado.
            unsafe { libc::close(read_fd); }

            // O child herda a arena via COW. Chama a Action diretamente.
            // A Action tem ABI: (fiber_arena, caller_arena, args_ptr) -> i64.
            // O child usa a arena herdada como ambas (fiber e caller).
            let action: extern "C" fn(i64, i64, i64) -> i64 =
                unsafe { std::mem::transmute(fn_ptr) };
            let result = action(arena_handle, arena_handle, args_ptr);

            // Serializa o resultado via to_bytes.
            let blob = kata_rt_to_bytes(result, result_type_id, arena_handle);

            // Escreve o blob no pipe. Primeiro o tamanho total, depois os bytes.
            // Layout do blob to_bytes: [len: i64, type_id: i64, rebase_count: i64,
            //                            rebase_offsets[rebase_count], data[len]]
            let blob_ptr = blob as *const u8;
            let total_len = unsafe { std::ptr::read_unaligned(blob_ptr as *const i64) };
            let rebase_count = unsafe { std::ptr::read_unaligned(blob_ptr.add(16) as *const i64) };
            let total_size = BLOB_HEADER_SIZE
                + (rebase_count as usize) * 8
                + (total_len as usize);

            // Escreve status OK + tamanho total + blob.
            let status = SPAWN_OK;
            let total_size_i64 = total_size as i64;
            let _ = io::stdout().flush();
            unsafe {
                libc::write(write_fd, &status as *const u8 as *const libc::c_void, 1);
                libc::write(
                    write_fd,
                    &total_size_i64 as *const i64 as *const libc::c_void,
                    8,
                );
                libc::write(write_fd, blob_ptr as *const libc::c_void, total_size);
            }

            // Fecha o pipe de escrita e termina.
            unsafe { libc::close(write_fd); }
            unsafe { libc::_exit(0); }
        }
        pid => {
            // ── PARENT ─────────────────────────────────────
            // Fecha o lado de escrita do pipe de resultado.
            unsafe { libc::close(write_fd); }

            // Yield — cede a outras fibers enquanto o child executa.
            crate::scheduler::kata_rt_yield();

            // Lê o status do child.
            let mut status = [0u8; 1];
            let n = unsafe {
                libc::read(read_fd, status.as_mut_ptr() as *mut libc::c_void, 1)
            };
            if n != 1 || status[0] != SPAWN_OK {
                unsafe { libc::close(read_fd); }
                // Reap zombie.
                unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0); }
                return 0;
            }

            // Lê o tamanho total do blob.
            let mut total_size_bytes = [0u8; 8];
            let n = unsafe {
                libc::read(
                    read_fd,
                    total_size_bytes.as_mut_ptr() as *mut libc::c_void,
                    8,
                )
            };
            if n != 8 {
                unsafe { libc::close(read_fd); }
                unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0); }
                return 0;
            }
            let total_size = i64::from_le_bytes(total_size_bytes) as usize;

            // Lê o blob completo.
            let mut blob_buf = vec![0u8; total_size];
            let mut read_total = 0;
            while read_total < total_size {
                let n = unsafe {
                    libc::read(
                        read_fd,
                        blob_buf[read_total..].as_mut_ptr() as *mut libc::c_void,
                        total_size - read_total,
                    )
                };
                if n <= 0 {
                    break;
                }
                read_total += n as usize;
            }
            unsafe { libc::close(read_fd); }

            // Reap zombie.
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0); }

            if read_total != total_size {
                return 0;
            }

            // O blob está na pilha do parent. from_bytes precisa de um
            // ponteiro estável — o Vec fornece isso. Desserializa na
            // arena do parent.
            let blob_ptr = blob_buf.as_ptr() as i64;
            kata_rt_from_bytes(blob_ptr, arena_handle)

            // blob_buf é dropped aqui — mas from_bytes já copiou os
            // dados para a arena. O ponteiro do Vec era temporário.
            // FIXME: from_bytes pode manter referências ao blob?
            // Verificar — se sim, precisa de leak ou cópia para a arena.
        }
    }
}