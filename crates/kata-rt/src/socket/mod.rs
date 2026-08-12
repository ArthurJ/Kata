//! Socket I/O — handle opaco para sockets TCP e Unix.
//!
//! Layout:
//! - `SocketInner` alocado via `kata_rt_arena_alloc` na root_arena (sem header ARC).
//!   Handle = ponteiro puro para SocketInner. O close faz `libc::close(fd)`
//!   e marca `closed = true`. Idempotente via campo `closed`.
//! - FD bruto (i32) armazenado diretamente — sem `TcpStream`/`UnixStream`.
//!   Isto permite poll uniforme (FD é FD) e non-blocking via fcntl.
//! - Result boxes alocados via `kata_rt_arena_alloc` na root_arena.
//! - Bytes de `read`/`read_chunk` alocados via `kata_rt_arena_alloc`.
//!
//! **Estrutura do módulo:** este arquivo (módulo pai) concentra as structs
//! (`SocketInner`, `SocketState`, `SocketKindRust`) e os helpers de
//! alocação/arena compartilhados. A FFI de criação/accept está em [`create`],
//! a FFI de read/write/close em [`io`], e o select/poll em [`select`].
//!
//! FFI:
//! - `kata_rt_socket_open(kind_box, mode_box) -> result_box` — Result::(Socket, Text)
//! - `kata_rt_socket_listen(listener_handle) -> result_box` — Result::(Socket, Text)
//! - `kata_rt_socket_read(handle) -> result_box` — Result::(Bytes, Text)
//! - `kata_rt_socket_read_chunk(handle, n) -> result_box` — Result::(Bytes, Text)
//! - `kata_rt_socket_readline(handle) -> result_box` — Result::(Text, Text)
//! - `kata_rt_socket_write_text(handle, data_ptr) -> result_box` — Result::(Unit, Text)
//! - `kata_rt_socket_write_bytes(handle, data_ptr) -> result_box` — Result::(Unit, Text)
//! - `kata_rt_socket_close(handle) -> ()` — fecha socket (idempotente)

pub(crate) mod create;
pub(crate) mod io;
pub(crate) mod select;

// Re-exports da camada de FFI — `lib.rs` continua importando os mesmos
// símbolos C-ABI.
pub use create::{kata_rt_socket_listen, kata_rt_socket_open};
pub use io::{
    kata_rt_socket_close, kata_rt_socket_read, kata_rt_socket_read_chunk, kata_rt_socket_readline,
    kata_rt_socket_write_bytes, kata_rt_socket_write_text,
};
// Re-exports pub(crate) para scheduler e channel::select.
pub(crate) use select::{SOCKET_WOULD_BLOCK, collect_socket_fds, try_select_sockets};

// ── Tipos ─────────────────────────────────────────────────────────

/// Estado do socket — determina quais operações são válidas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SocketState {
    Listener,
    Connected,
}

/// Tipo do socket (TCP ou Unix) — para close correto e validação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SocketKindRust {
    Tcp,
    Unix,
}

/// SocketInner — socket aberto com estado, FD bruto e endereço.
/// Alocado via `arena_alloc` na root_arena.
///
/// O descritor OS é armazenado como FD bruto (i32), não como
/// `TcpStream`/`UnixStream`. Isto permite:
/// - Extrair FD para `poll` uniformemente (igual FileInner)
/// - Non-blocking controlado via `fcntl(F_SETFL, O_NONBLOCK)`
/// - `read`/`write` via syscall direta (sem BufReader)
pub(crate) struct SocketInner {
    pub closed: bool,
    pub fd: i32,
    pub state: SocketState,
    pub kind: SocketKindRust,
    /// Endereço do socket — guardado para diagnóstico/debug. Não lido em
    /// operações de I/O, mas útil para logging futuro.
    #[allow(dead_code)]
    pub addr: String,
    /// Buffer parcial para readline — acumula bytes até encontrar \n.
    /// Usado apenas por `kata_rt_socket_readline`; `read`/`read_chunk`
    /// lêem do FD diretamente. Não misturar readline com read/read_chunk
    /// no mesmo socket (mesma limitação de Go bufio / Rust BufReader).
    pub line_buf: Vec<u8>,
}

// ── Helpers compartilhados ──────────────────────────────────────────

/// Aloca um bloco na root_arena via `kata_rt_arena_alloc`.
pub(crate) fn arena_alloc(size: i64) -> i64 {
    let rt = crate::arena::rt_ptr();
    let root_arena = crate::arena::kata_rt_get_root_arena_handle(rt);
    crate::arena::kata_rt_arena_alloc(rt, root_arena, size)
}

/// Aloca um Result box com tag e payload.
/// Layout: tag (i64) no offset 0, payload (i64) no offset 8.
pub(crate) fn alloc_result_box(tag: i64, payload: i64) -> i64 {
    let data_ptr = arena_alloc(16);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut i64, tag);
        std::ptr::write_unaligned((data_ptr as *mut u8).add(8) as *mut i64, payload);
    }
    data_ptr
}

/// Aloca um SocketInner e retorna o ponteiro (handle).
pub(crate) fn alloc_socket_inner(inner: SocketInner) -> i64 {
    let size = std::mem::size_of::<SocketInner>() as i64;
    let data_ptr = arena_alloc(size);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut SocketInner, inner);
    }
    data_ptr
}

/// Extrai `SocketInner` de um handle (ponteiro puro).
/// Retorna `None` se o handle é 0 (nulo).
pub(crate) fn socket_from_handle(handle: i64) -> Option<&'static mut SocketInner> {
    if handle == 0 {
        return None;
    }
    Some(unsafe { &mut *(handle as *mut SocketInner) })
}

/// Cria um Text a partir de uma String (C string nulo-terminada).
pub(crate) fn alloc_text(s: &str) -> i64 {
    let data_size = s.len() as i64 + 1;
    let data_ptr = arena_alloc(data_size);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), data_ptr as *mut u8, s.len());
        std::ptr::write_unaligned((data_ptr as *mut u8).add(s.len()), 0);
    }
    data_ptr
}

/// Cria um Bytes a partir de um Vec<u8>.
/// Layout: len (i64) no offset 0, data[i] no offset 8+i.
pub(crate) fn alloc_bytes(data: &[u8]) -> i64 {
    let data_size = 8 + data.len() as i64;
    let data_ptr = arena_alloc(data_size);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut i64, data.len() as i64);
        if !data.is_empty() {
            std::ptr::copy_nonoverlapping(data.as_ptr(), (data_ptr as *mut u8).add(8), data.len());
        }
    }
    data_ptr
}

/// Cria uma mensagem de erro como Text.
pub(crate) fn error_text(msg: &str) -> i64 {
    alloc_text(msg)
}

/// Configura FD como non-blocking via fcntl(F_SETFL, O_NONBLOCK).
pub(crate) use crate::platform::set_nonblocking;

/// Habilita SO_REUSEADDR no socket (para reiniciar servidor após crash).
pub(crate) use crate::platform::set_reuseaddr;
