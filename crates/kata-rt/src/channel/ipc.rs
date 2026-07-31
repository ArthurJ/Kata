//! Canal cross-process via Unix pipe.
//!
//! Estrutura alocada na arena que carrega os FDs de um pipe Unix.
//! O `send` serializa o valor com `to_bytes` e escreve no pipe; o `recv`
//! lê o blob do pipe e desserializa com `from_bytes`.
//!
//! **Lifetime:** "último apaga a luz". Cada processo fecha o FD que não
//! usa. O kernel rastreia referências por FD. Quando a arena é destruída
//! (fiber completa → `bump.reset()`), o struct é descartado — mas o FD
//! não é fechado automaticamente pelo bumpalo (não chama destructors).
//! O caller é responsável por fechar FDs explicitamente quando souber
//! que não precisa mais do canal. O child fecha seu FD no `_exit(0)`.
//!
//! **Blocking cooperativo:** `recv` usa `poll(read_fd, POLLIN, timeout=0)`
//! para checar dados sem bloquear. Se não há dados e há fiber em execução,
//! suspende com `YieldReason::WaitingOnChannel(handle)`. Quando resumido,
//! tenta novamente. `send` faz `write` blocking — o pipe buffer do kernel
//! (64KB no Linux) acomoda a maioria dos blobs. Se o buffer enche, `send`
//! bloqueia no `write` (limitação v1 — futuro: non-blocking write + yield).

use super::ops::{OK, WOULD_BLOCK};
use super::{IpcChannelInner, TAG_IPC_CHANNEL, arena_alloc_and_init, make_handle_pub, ptr_of};

/// Cria um canal cross-process: aloca um pipe Unix, guarda os FDs no
/// `IpcChannelInner` alocado na arena. Retorna handle com tag
/// `TAG_IPC_CHANNEL` (0b100).
///
/// O `type_id` é o índice na type table do tipo de elemento do canal.
/// Usado por `to_bytes`/`from_bytes` para serializar/desserializar.
///
/// **Fork:** o pipe é herdado por ambos os processos. O parent e o child
/// cada um fecha o FD que não precisa após o fork (feito no codegen/runtime
/// do spawn, não aqui).
///
/// # Safety
/// `arena` deve ser um handle válido.
pub(super) unsafe fn create_ipc_channel(arena: i64, type_id: i64) -> i64 {
    let mut fds = [0i32; 2];
    // SAFETY: pipe() cria dois FDs. Retorna 0 em sucesso, -1 em erro.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        return 0;
    }
    let inner = IpcChannelInner {
        write_fd: fds[1],
        read_fd: fds[0],
        type_id,
    };
    // SAFETY: arena é válido (contrato FFI).
    let ptr = unsafe { arena_alloc_and_init(arena, inner) };
    if ptr.is_null() {
        // Não fecha os FDs — o caller não os tem. Vazamento esperado em erro.
        return 0;
    }
    make_handle_pub(ptr, TAG_IPC_CHANNEL)
}

/// Tenta enviar um valor serializado pelo canal IPC.
///
/// Serializa `value` com `to_bytes(value, type_id, arena)`, escreve o
/// blob no pipe (len prefix + bytes). Retorna `OK` (0) se completou.
///
/// `arena` para `to_bytes` — usa a root_arena se disponível, senão 0.
///
/// # Safety
/// `handle` deve ser um handle IPC válido. `value` deve ser um valor
/// runtime válido do tipo correspondente ao `type_id` do canal.
pub(super) unsafe fn try_ipc_send(handle: i64, value: i64) -> i64 {
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return WOULD_BLOCK;
    }
    // SAFETY: handle veio de create_ipc_channel. Ponteiro válido na arena.
    let inner = unsafe { &*(ptr as *const IpcChannelInner) };

    // Serializa o valor em um blob Bytes. Usa root_arena (TLS) — canais
    // são criados dentro de Actions onde root_arena já está inicializada.
    let arena = crate::arena::kata_rt_get_root_arena_handle();
    let blob = crate::marshal::kata_rt_to_bytes(value, inner.type_id, arena);
    if blob == 0 {
        return WOULD_BLOCK;
    }

    // Lê o content_len do blob (header: 8 bytes = content_len).
    let content_len = unsafe { std::ptr::read_unaligned(blob as *const i64) };
    if content_len <= 0 {
        return WOULD_BLOCK;
    }

    // Escreve: [content_len: 8 bytes][content: content_len bytes]
    // O blob já tem este layout exato (8 + content_len bytes).
    let total = 8 + content_len;
    let blob_ptr = blob as *const u8;
    // SAFETY: blob é válido, total bytes.
    let slice = unsafe { std::slice::from_raw_parts(blob_ptr, total as usize) };

    // SAFETY: write_fd é um FD válido de pipe. write é blocking.
    let mut written = 0usize;
    while written < slice.len() {
        let n = unsafe {
            libc::write(
                inner.write_fd,
                slice.as_ptr().add(written) as *const libc::c_void,
                (slice.len() - written) as libc::size_t,
            )
        };
        if n < 0 {
            // Erro de escrita.
            return WOULD_BLOCK;
        }
        written += n as usize;
    }
    OK
}

/// Tenta receber um valor do canal IPC.
///
/// Lê o blob do pipe (len prefix + bytes), desserializa com `from_bytes`
/// na arena fornecida. Retorna o valor desserializado ou `WOULD_BLOCK`
/// se não há dados disponíveis.
///
/// # Safety
/// `handle` deve ser um handle IPC válido. `arena` deve ser válido.
pub(super) unsafe fn try_ipc_recv(handle: i64, arena: i64) -> i64 {
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return WOULD_BLOCK;
    }
    // SAFETY: handle veio de create_ipc_channel.
    let inner = unsafe { &*(ptr as *const IpcChannelInner) };

    // poll(read_fd, POLLIN, timeout=0) — non-blocking check.
    let mut pfd = libc::pollfd {
        fd: inner.read_fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll com 1 FD e timeout 0 é non-blocking.
    let rc = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, 0) };
    if rc <= 0 || (pfd.revents & libc::POLLIN) == 0 {
        // Sem dados disponíveis.
        return WOULD_BLOCK;
    }

    // Há dados — lê o content_len (8 bytes) em modo blocking.
    let mut len_buf = [0u8; 8];
    let mut read_total = 0usize;
    while read_total < 8 {
        let n = unsafe {
            libc::read(
                inner.read_fd,
                len_buf.as_mut_ptr().add(read_total) as *mut libc::c_void,
                (8 - read_total) as libc::size_t,
            )
        };
        if n <= 0 {
            return WOULD_BLOCK;
        }
        read_total += n as usize;
    }
    let content_len = i64::from_le_bytes(len_buf);
    if content_len <= 0 {
        return WOULD_BLOCK;
    }

    // Lê o restante do blob (content_len bytes).
    let total = content_len as usize;
    let mut content = vec![0u8; total];
    read_total = 0usize;
    while read_total < total {
        let n = unsafe {
            libc::read(
                inner.read_fd,
                content.as_mut_ptr().add(read_total) as *mut libc::c_void,
                (total - read_total) as libc::size_t,
            )
        };
        if n <= 0 {
            return WOULD_BLOCK;
        }
        read_total += n as usize;
    }

    // Reconstrói o blob completo (8 bytes header + content) em uma
    // alocação e chama from_bytes.
    let blob_size = 8 + total;
    let blob_ptr = crate::arena::kata_rt_arena_alloc(arena, blob_size as i64);
    if blob_ptr == 0 {
        return WOULD_BLOCK;
    }
    unsafe {
        let p = blob_ptr as *mut u8;
        std::ptr::copy_nonoverlapping(len_buf.as_ptr(), p, 8);
        std::ptr::copy_nonoverlapping(content.as_ptr(), p.add(8), total);
    }

    crate::marshal::kata_rt_from_bytes(blob_ptr, arena)
}

/// Verifica se `kata_rt_channel_recv` retornaria um valor (não WOULD_BLOCK)
/// **sem consumir o valor**. Usado pelo scheduler no wake pass.
///
/// Usa `poll(read_fd, POLLIN, timeout=0)` — non-blocking.
///
/// # Safety
/// `handle` deve ser um handle IPC válido.
pub(super) unsafe fn can_ipc_recv(handle: i64) -> bool {
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return false;
    }
    // SAFETY: handle veio de create_ipc_channel.
    let inner = unsafe { &*(ptr as *const IpcChannelInner) };
    let mut pfd = libc::pollfd {
        fd: inner.read_fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll non-blocking (timeout=0).
    let rc = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, 0) };
    rc > 0 && (pfd.revents & libc::POLLIN) != 0
}

/// Verifica se `kata_rt_channel_send` retornaria OK (não WOULD_BLOCK)
/// **sem enviar**. Para pipe, assume sempre OK (buffer do kernel).
///
/// # Safety
/// `handle` deve ser um handle IPC válido.
pub(super) unsafe fn can_ipc_send(_handle: i64) -> bool {
    // Pipe write blocking: assume sempre OK (kernel buffer > 0).
    true
}

/// Bloqueia (blocking poll) até que o read_fd tenha dados disponíveis.
/// Usado pelo child após `spawn!` — o child não tem scheduler/fiber, então
/// `kata_rt_channel_recv` não pode suspender. Em vez de retornar
/// `WOULD_BLOCK` imediatamente, faz `poll(fd, POLLIN, timeout=-1)` que
/// bloqueia até o parent escrever no pipe.
///
/// # Safety
/// `handle` deve ser um handle IPC válido.
pub(super) unsafe fn block_until_readable(handle: i64) {
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return;
    }
    let inner = unsafe { &*(ptr as *const IpcChannelInner) };
    let mut pfd = libc::pollfd {
        fd: inner.read_fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll com timeout -1 (infinite) bloqueia até POLLIN ou POLLHUP.
    unsafe {
        libc::poll(&mut pfd as *mut libc::pollfd, 1, -1);
    }
}

/// Fecha um FD de escrita do canal IPC. Chamado pelo parent após
/// fork para sinalizar que não vai enviar mais dados pelo canal.
///
/// # Safety
/// `handle` deve ser um handle IPC válido.
#[allow(dead_code)] // usado na integração spawn! + IPC
pub(super) unsafe fn ipc_close_write(handle: i64) {
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return;
    }
    let inner = unsafe { &*(ptr as *const IpcChannelInner) };
    unsafe {
        libc::close(inner.write_fd);
    }
}

/// Fecha um FD de leitura do canal IPC. Chamado pelo child após
/// fork para sinalizar que não vai receber dados pelo canal.
///
/// # Safety
/// `handle` deve ser um handle IPC válido.
#[allow(dead_code)] // usado na integração spawn! + IPC
pub(super) unsafe fn ipc_close_read(handle: i64) {
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return;
    }
    let inner = unsafe { &*(ptr as *const IpcChannelInner) };
    unsafe {
        libc::close(inner.read_fd);
    }
}
