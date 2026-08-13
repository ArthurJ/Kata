//! File handle selection — helpers de scheduler para file descriptors.
//!
//! Responsabilidade: dado um slice de file handles, determinar quais estão
//! prontos para leitura (via `poll` non-blocking). Usado pelo scheduler no
//! `wake_pass` e no `channel/select.rs` para multiplexação.
//!
//! Layout:
//! - `FILE_WOULD_BLOCK`: sentinel retornado quando nenhum handle está pronto.
//! - `try_select_files`: poll non-blocking, retorna índice do primeiro pronto.
//! - `collect_file_fds`: coleta FDs brutos para poll unificado (sleep path).
//! - `kata_rt_select_files`: FFI de select multiplexado com suspensão de fiber.

use crate::platform::{POLLHUP, POLLIN, PollFd, file_raw_fd, poll_fds};

use super::FileInner;

// ── Select para File handles ──────────────────────────────────────

/// Sentinel: nenhum handle pronto (mesmo valor que WOULD_BLOCK de canais).
pub(crate) const FILE_WOULD_BLOCK: i64 = -1;

/// Tenta encontrar um file handle pronto para leitura (non-blocking).
///
/// Usa `poll(POLLIN, timeout=0)` — para arquivos regulares, sempre retorna
/// "pronto". Para pipes/sockets, verifica readiness real.
///
/// Retorna o índice (0..N-1) do primeiro handle pronto, ou `FILE_WOULD_BLOCK`.
///
/// Função interna — chamada pelo scheduler no `wake_pass`.
pub(crate) fn try_select_files(handles: &[i64]) -> i64 {
    if handles.is_empty() {
        return FILE_WOULD_BLOCK;
    }

    // Coleta os FDs dos file handles.
    let mut fds: Vec<PollFd> = Vec::with_capacity(handles.len());
    let mut valid_indices: Vec<usize> = Vec::with_capacity(handles.len());

    for (idx, &handle) in handles.iter().enumerate() {
        if handle == 0 {
            continue;
        }
        // SAFETY: handle foi criado por alloc_file_inner (arena alloc).
        let inner = unsafe { &*(handle as *const FileInner) };
        if inner.closed {
            continue;
        }
        let fd = file_raw_fd(inner.buf_reader.get_ref());
        fds.push(PollFd {
            fd,
            events: POLLIN,
            revents: 0,
        });
        valid_indices.push(idx);
    }

    if fds.is_empty() {
        return FILE_WOULD_BLOCK;
    }

    // poll non-blocking (timeout=0).
    let ret = poll_fds(&mut fds, 0);
    if ret <= 0 {
        return FILE_WOULD_BLOCK;
    }

    // Encontra o primeiro FD pronto.
    for (i, pfd) in fds.iter().enumerate() {
        if pfd.revents & (POLLIN | POLLHUP) != 0 {
            return valid_indices[i] as i64;
        }
    }

    FILE_WOULD_BLOCK
}

/// Coleta os FDs brutos de file handles válidos e não-fechados.
/// Usado pelo scheduler no sleep path para poll unificado (IPC + files).
///
/// Retorna `(fds, valid_indices)` — os FDs para poll e os índices originais
/// correspondentes (necessário para mapear de volta se preciso).
pub(crate) fn collect_file_fds(handles: &[i64]) -> Vec<PollFd> {
    let mut fds: Vec<PollFd> = Vec::with_capacity(handles.len());

    for &handle in handles {
        if handle == 0 {
            continue;
        }
        // SAFETY: handle foi criado por alloc_file_inner (arena alloc).
        let inner = unsafe { &*(handle as *const FileInner) };
        if inner.closed {
            continue;
        }
        let fd = file_raw_fd(inner.buf_reader.get_ref());
        fds.push(PollFd {
            fd,
            events: POLLIN,
            revents: 0,
        });
    }

    fds
}

/// Select multiplexado para file handles.
///
/// Retorna o índice (0..N-1) do primeiro file handle pronto para leitura.
/// O codegen então chama `kata_rt_file_read_chunk(handle, n)` para ler.
///
/// - Retorna `FILE_WOULD_BLOCK` (-1) se nenhum handle está pronto e
///   chamado fora de fiber.
/// - Suspende o fiber com `WaitingOnSelect` se chamado dentro de fiber.
///
/// **Blocking cooperativo:** se nenhum file handle tem dado e há um fiber
/// em execução, suspende o fiber. O scheduler acorda o fiber quando algum
/// FD tem dado.
///
/// # Safety
/// `handles` deve apontar para um array de `n_handles` handles de File válidos.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn kata_rt_select_files(handles: *const i64, n_handles: i64) -> i64 {
    if handles.is_null() || n_handles <= 0 {
        return FILE_WOULD_BLOCK;
    }
    // SAFETY: handles é um ponteiro válido para n_handles i64s (contrato FFI).
    let handles_slice = unsafe { std::slice::from_raw_parts(handles, n_handles as usize) };
    let file_handles_vec: Vec<i64> = handles_slice.to_vec();

    loop {
        // 1. Tentar todos os file handles (non-blocking poll).
        let result = try_select_files(handles_slice);
        if result != FILE_WOULD_BLOCK {
            return result;
        }

        // 2. Suspende o fiber com WaitingOnSelect (sem channel handles,
        //    sem timeout — o codegen trata timeout separadamente).
        let suspended = crate::fiber::with_suspend(|suspend| {
            suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                channel_handles: Vec::new(),
                file_handles: file_handles_vec.clone(),
                socket_handles: Vec::new(),
                deadline: None,
            });
        });
        if suspended.is_none() {
            // Fora de fiber (teste unitário) — retorna WOULD_BLOCK.
            return FILE_WOULD_BLOCK;
        }
        // Fiber resumido — scheduler acredita que há dado.
        // Loop tenta novamente.
    }
}
