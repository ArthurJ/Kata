//! Select/poll para socket handles — usado pelo scheduler e `channel::select`.
//!
//! `try_select_sockets` faz poll non-blocking (timeout=0) nos FDs dos sockets.
//! `collect_socket_fds` coleta pollfds para o sleep path unificado do scheduler.

use crate::platform::{poll_fds, PollFd, POLLHUP, POLLIN, POLLOUT};

use super::SocketInner;

/// Sentinel: nenhum handle pronto.
pub(crate) const SOCKET_WOULD_BLOCK: i64 = -1;

/// Tenta encontrar um socket handle pronto para I/O (non-blocking).
///
/// Usa `poll(POLLIN | POLLOUT, timeout=0)` — sockets podem bloquear
/// tanto em read quanto write.
///
/// Retorna o índice (0..N-1) do primeiro handle pronto, ou `SOCKET_WOULD_BLOCK`.
pub(crate) fn try_select_sockets(handles: &[i64]) -> i64 {
    if handles.is_empty() {
        return SOCKET_WOULD_BLOCK;
    }

    let mut fds: Vec<PollFd> = Vec::with_capacity(handles.len());
    let mut valid_indices: Vec<usize> = Vec::with_capacity(handles.len());

    for (idx, &handle) in handles.iter().enumerate() {
        if handle == 0 {
            continue;
        }
        let inner = unsafe { &*(handle as *const SocketInner) };
        if inner.closed {
            continue;
        }
        fds.push(PollFd {
            fd: inner.fd,
            events: POLLIN | POLLOUT,
            revents: 0,
        });
        valid_indices.push(idx);
    }

    if fds.is_empty() {
        return SOCKET_WOULD_BLOCK;
    }

    let ret = poll_fds(&mut fds, 0);
    if ret <= 0 {
        return SOCKET_WOULD_BLOCK;
    }

    for (i, pfd) in fds.iter().enumerate() {
        if pfd.revents & (POLLIN | POLLOUT | POLLHUP) != 0 {
            return valid_indices[i] as i64;
        }
    }

    SOCKET_WOULD_BLOCK
}

/// Coleta os FDs brutos de socket handles válidos e não-fechados.
/// Usado pelo scheduler no sleep path para poll unificado.
/// Inclui POLLIN | POLLOUT — sockets podem bloquear tanto em read quanto write.
pub(crate) fn collect_socket_fds(handles: &[i64]) -> Vec<PollFd> {
    let mut fds: Vec<PollFd> = Vec::with_capacity(handles.len());

    for &handle in handles {
        if handle == 0 {
            continue;
        }
        let inner = unsafe { &*(handle as *const SocketInner) };
        if inner.closed {
            continue;
        }
        fds.push(PollFd {
            fd: inner.fd,
            events: POLLIN | POLLOUT,
            revents: 0,
        });
    }

    fds
}
