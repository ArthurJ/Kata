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
//! FFI:
//! - `kata_rt_socket_open(kind_box, mode_box) -> result_box` — Result::(Socket, Text)
//! - `kata_rt_socket_listen(listener_handle) -> result_box` — Result::(Socket, Text)
//! - `kata_rt_socket_read(handle) -> result_box` — Result::(Bytes, Text)
//! - `kata_rt_socket_read_chunk(handle, n) -> result_box` — Result::(Bytes, Text)
//! - `kata_rt_socket_write_text(handle, data_ptr) -> result_box` — Result::(Unit, Text)
//! - `kata_rt_socket_write_bytes(handle, data_ptr) -> result_box` — Result::(Unit, Text)
//! - `kata_rt_socket_close(handle) -> ()` — fecha socket (idempotente)

use std::ffi::CStr;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::raw::c_char;
use std::os::unix::io::{AsRawFd, IntoRawFd};
use std::os::unix::net::UnixListener;

// ── SocketInner ────────────────────────────────────────────────────

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
    pub addr: String,
}

// ── Helpers ────────────────────────────────────────────────────────

/// Aloca um bloco na root_arena via `kata_rt_arena_alloc`.
fn arena_alloc(size: i64) -> i64 {
    let root_arena = crate::arena::kata_rt_get_root_arena_handle();
    if root_arena < 0 {
        return 0;
    }
    crate::arena::kata_rt_arena_alloc(root_arena, size)
}

/// Aloca um Result box com tag e payload.
/// Layout: tag (i64) no offset 0, payload (i64) no offset 8.
fn alloc_result_box(tag: i64, payload: i64) -> i64 {
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
fn alloc_socket_inner(inner: SocketInner) -> i64 {
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
fn socket_from_handle(handle: i64) -> Option<&'static mut SocketInner> {
    if handle == 0 {
        return None;
    }
    Some(unsafe { &mut *(handle as *mut SocketInner) })
}

/// Cria um Text a partir de uma String (C string nulo-terminada).
fn alloc_text(s: &str) -> i64 {
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
fn alloc_bytes(data: &[u8]) -> i64 {
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
fn error_text(msg: &str) -> i64 {
    alloc_text(msg)
}

/// Configura FD como non-blocking via fcntl(F_SETFL, O_NONBLOCK).
fn set_nonblocking(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// Habilita SO_REUSEADDR no socket (para reiniciar servidor após crash).
fn set_reuseaddr(fd: i32) {
    let optval: i32 = 1;
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &optval as *const _ as *const libc::c_void,
            std::mem::size_of::<i32>() as libc::socklen_t,
        );
    }
}

// ── FFI ────────────────────────────────────────────────────────────

/// Cria um socket (TCP ou Unix, Listener ou Connected) e retorna Result box.
///
/// `kind_box` é um Sum box SocketKind (tag 0=TCP, 1=Unix, payload Text = endereço).
/// `mode_box` é um Sum box SocketMode (tag 0=Listener, 1=Connected).
///
/// Retorna:
/// - Result box Ok(handle) se sucesso — handle é ponteiro para SocketInner.
/// - Result box Err(text) se erro.
///
/// # Safety
/// `kind_box` e `mode_box` devem ser ponteiros válidos para Sum boxes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_open(kind_box: i64, mode_box: i64) -> i64 {
    // Extrai kind: tag (0=TCP, 1=Unix) + payload (Text = C string no offset 8).
    let kind_tag = if kind_box == 0 {
        0
    } else {
        crate::sum::kata_rt_sum_tag_int(kind_box)
    };
    let addr_ptr: i64 = if kind_box == 0 {
        0
    } else {
        unsafe { std::ptr::read_unaligned((kind_box as *const u8).add(8) as *const i64) }
    };
    let addr = if addr_ptr == 0 {
        String::new()
    } else {
        unsafe { CStr::from_ptr(addr_ptr as *const c_char) }
            .to_string_lossy()
            .to_string()
    };

    // Extrai mode: tag (0=Listener, 1=Connected).
    let mode_tag = if mode_box == 0 {
        0
    } else {
        crate::sum::kata_rt_sum_tag_int(mode_box)
    };

    match (kind_tag, mode_tag) {
        (0, 0) => create_tcp_listener(&addr),
        (0, 1) => create_tcp_connected(&addr),
        (1, 0) => create_unix_listener(&addr),
        (1, 1) => create_unix_connected(&addr),
        _ => alloc_result_box(1, error_text("kind/mode inválido")),
    }
}

/// Cria listener TCP: bind + listen, configura non-blocking + SO_REUSEADDR.
fn create_tcp_listener(addr: &str) -> i64 {
    let sock_addr: SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(e) => return alloc_result_box(1, error_text(&format!("endereço inválido: {e}"))),
    };

    let listener = match TcpListener::bind(sock_addr) {
        Ok(l) => l,
        Err(e) => return alloc_result_box(1, error_text(&format!("bind falhou: {e}"))),
    };

    set_reuseaddr(listener.as_raw_fd());
    set_nonblocking(listener.as_raw_fd());

    let fd = listener.into_raw_fd();

    let inner = SocketInner {
        closed: false,
        fd,
        state: SocketState::Listener,
        kind: SocketKindRust::Tcp,
        addr: sock_addr.to_string(),
    };
    let handle = alloc_socket_inner(inner);
    if handle == 0 {
        unsafe { libc::close(fd) };
        return alloc_result_box(1, error_text("falha na alocação"));
    }
    alloc_result_box(0, handle)
}

/// Cria socket TCP conectado: connect blocking com timeout.
///
/// Usa `TcpStream::connect_timeout` do Rust std (blocking). Se o servidor não
/// estiver ouvindo, suspende o fiber e tenta novamente (o servidor pode ainda
/// não ter feito listen, especialmente em testes com fork!).
fn create_tcp_connected(addr: &str) -> i64 {
    let sock_addr: SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(e) => return alloc_result_box(1, error_text(&format!("endereço inválido: {e}"))),
    };

    // Ceder controle ao scheduler antes do primeiro connect — o servidor
    // (fork!) pode ainda não ter executado bind+listen. Sem isto, o primeiro
    // connect_timeout bloqueia a thread e o servidor nunca roda.
    let _ = crate::fiber::with_suspend(|suspend| {
        suspend.suspend(crate::fiber::YieldReason::Sleep(
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        ));
    });

    // Tentar connect com retry cooperativo. Se ECONNREFUSED, suspender o fiber
    // e tentar novamente (o servidor pode ainda não ter feito listen).
    let max_retries = 50;
    for _ in 0..max_retries {
        match TcpStream::connect_timeout(&sock_addr, std::time::Duration::from_millis(200)) {
            Ok(stream) => {
                set_nonblocking(stream.as_raw_fd());
                let fd = stream.into_raw_fd();
                let inner = SocketInner {
                    closed: false,
                    fd,
                    state: SocketState::Connected,
                    kind: SocketKindRust::Tcp,
                    addr: sock_addr.to_string(),
                };
                let handle = alloc_socket_inner(inner);
                if handle == 0 {
                    unsafe { libc::close(fd) };
                    return alloc_result_box(1, error_text("falha na alocação"));
                }
                return alloc_result_box(0, handle);
            }
            Err(e) => {
                // ECONNREFUSED — servidor não está ouvindo ainda. Suspende e tenta de novo.
                let suspended = crate::fiber::with_suspend(|suspend| {
                    suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                        channel_handles: Vec::new(),
                        file_handles: Vec::new(),
                        socket_handles: Vec::new(),
                        deadline: Some(
                            std::time::Instant::now() + std::time::Duration::from_millis(100),
                        ),
                    });
                });
                if suspended.is_none() {
                    return alloc_result_box(1, error_text(&format!("connect falhou: {e}")));
                }
                // Após resume, tentar novamente.
            }
        }
    }
    alloc_result_box(1, error_text("connect falhou: timeout após retries"))
}

/// Cria listener Unix domain socket: bind + listen, non-blocking.
fn create_unix_listener(path: &str) -> i64 {
    // Remove socket file anterior se existir (para reiniciar servidor).
    let _ = std::fs::remove_file(path);

    let listener = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => return alloc_result_box(1, error_text(&format!("bind unix falhou: {e}"))),
    };

    set_nonblocking(listener.as_raw_fd());

    let fd = listener.into_raw_fd();

    let inner = SocketInner {
        closed: false,
        fd,
        state: SocketState::Listener,
        kind: SocketKindRust::Unix,
        addr: path.to_string(),
    };
    let handle = alloc_socket_inner(inner);
    if handle == 0 {
        unsafe { libc::close(fd) };
        return alloc_result_box(1, error_text("falha na alocação"));
    }
    alloc_result_box(0, handle)
}

/// Cria socket Unix conectado: connect com retry cooperativo.
fn create_unix_connected(path: &str) -> i64 {
    // Ceder controle ao scheduler — o servidor (fork!) pode ainda não ter
    // feito bind+listen.
    let _ = crate::fiber::with_suspend(|suspend| {
        suspend.suspend(crate::fiber::YieldReason::Sleep(
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        ));
    });

    let max_retries = 50;
    for _ in 0..max_retries {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(stream) => {
                set_nonblocking(stream.as_raw_fd());
                let fd = stream.into_raw_fd();
                let inner = SocketInner {
                    closed: false,
                    fd,
                    state: SocketState::Connected,
                    kind: SocketKindRust::Unix,
                    addr: path.to_string(),
                };
                let handle = alloc_socket_inner(inner);
                if handle == 0 {
                    unsafe { libc::close(fd) };
                    return alloc_result_box(1, error_text("falha na alocação"));
                }
                return alloc_result_box(0, handle);
            }
            Err(e) => {
                // Servidor não está ouvindo ainda. Suspende e tenta de novo.
                let suspended = crate::fiber::with_suspend(|suspend| {
                    suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                        channel_handles: Vec::new(),
                        file_handles: Vec::new(),
                        socket_handles: Vec::new(),
                        deadline: Some(
                            std::time::Instant::now() + std::time::Duration::from_millis(100),
                        ),
                    });
                });
                if suspended.is_none() {
                    return alloc_result_box(1, error_text(&format!("connect unix falhou: {e}")));
                }
            }
        }
    }
    alloc_result_box(1, error_text("connect unix falhou: timeout após retries"))
}

/// Aceita uma conexão no listener (non-blocking com suspensão cooperativa).
///
/// Retorna Result box Ok(connected_handle) ou Err(text).
///
/// # Safety
/// `listener_handle` deve ser um handle válido criado por `kata_rt_socket_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_listen(listener_handle: i64) -> i64 {
    let inner = match socket_from_handle(listener_handle) {
        Some(s) => s,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if inner.state != SocketState::Listener {
        return alloc_result_box(1, error_text("socket conectado não aceita conexões"));
    }

    // accept4 com SOCK_NONBLOCK — não precisa fcntl depois.
    let mut client_addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut addr_len: libc::socklen_t =
        std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;

    loop {
        let client_fd = unsafe {
            libc::accept4(
                inner.fd,
                &mut client_addr as *mut _ as *mut libc::sockaddr,
                &mut addr_len,
                libc::SOCK_NONBLOCK,
            )
        };

        if client_fd >= 0 {
            let client_inner = SocketInner {
                closed: false,
                fd: client_fd,
                state: SocketState::Connected,
                kind: inner.kind,
                addr: String::new(), // peer addr — não necessário para operação
            };
            let handle = alloc_socket_inner(client_inner);
            if handle == 0 {
                unsafe { libc::close(client_fd) };
                return alloc_result_box(1, error_text("falha na alocação"));
            }
            return alloc_result_box(0, handle);
        }

        // Erro — verificar se é EAGAIN/EWOULDBLOCK (non-blocking, sem conexão pendente).
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
            // Sem conexão pendente — suspender fiber, scheduler poll.
            // O handle do listener vai em socket_handles para o scheduler
            // fazer poll(POLLIN) no FD do listener.
            let suspended = crate::fiber::with_suspend(|suspend| {
                suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                    channel_handles: Vec::new(),
                    file_handles: Vec::new(),
                    socket_handles: vec![listener_handle],
                    deadline: None,
                });
            });
            if suspended.is_none() {
                // Fora de fiber (teste unitário) — retorna Err.
                return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
            }
            // Fiber resumido — tentar novamente.
            continue;
        }

        // Erro real de accept.
        return alloc_result_box(1, error_text(&format!("accept falhou: {err}")));
    }
}

/// Lê todo o dado disponível do socket como Bytes (slurp).
///
/// Non-blocking: se não há dados (EAGAIN), suspende o fiber.
/// EOF (read retorna 0) → Err("EOF").
///
/// # Safety
/// `handle` deve ser um handle válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_read(handle: i64) -> i64 {
    let inner = match socket_from_handle(handle) {
        Some(s) => s,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if inner.state != SocketState::Connected {
        return alloc_result_box(1, error_text("socket listener não suporta read"));
    }

    let mut data = Vec::new();
    let mut buf = [0u8; 8192];

    loop {
        let n_read =
            unsafe { libc::read(inner.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n_read > 0 {
            data.extend_from_slice(&buf[..n_read as usize]);
            // Continua lendo enquanto há dados (non-blocking).
            continue;
        }
        if n_read == 0 {
            // EOF — peer fechou.
            if data.is_empty() {
                return alloc_result_box(1, error_text("EOF"));
            }
            break;
        }
        // n_read < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
            if data.is_empty() {
                // Sem dados — suspender fiber com o handle em socket_handles
                // para o scheduler fazer poll(POLLIN) no FD do socket.
                let suspended = crate::fiber::with_suspend(|suspend| {
                    suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                        channel_handles: Vec::new(),
                        file_handles: Vec::new(),
                        socket_handles: vec![handle],
                        deadline: None,
                    });
                });
                if suspended.is_none() {
                    return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
                }
                // Fiber resumido — tentar novamente.
                continue;
            }
            // Já tem dados — retorna o que leu (partial read).
            break;
        }
        // Erro real.
        return alloc_result_box(1, error_text(&format!("erro de leitura: {err}")));
    }

    let bytes_ptr = alloc_bytes(&data);
    if bytes_ptr == 0 {
        return alloc_result_box(1, error_text("falha na alocação"));
    }
    alloc_result_box(0, bytes_ptr)
}

/// Lê até `n` bytes do socket como Bytes (chunk).
///
/// `n` é SMI-tagged (payload = n >> 1).
/// Non-blocking: se não há dados (EAGAIN), suspende o fiber.
/// EOF (read retorna 0) → Err("EOF").
///
/// # Safety
/// `handle` deve ser um handle válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_read_chunk(handle: i64, n: i64) -> i64 {
    let inner = match socket_from_handle(handle) {
        Some(s) => s,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if inner.state != SocketState::Connected {
        return alloc_result_box(1, error_text("socket listener não suporta read"));
    }

    let max_bytes = (n >> 1) as usize;
    let mut buf = vec![0u8; max_bytes];

    loop {
        let n_read =
            unsafe { libc::read(inner.fd, buf.as_mut_ptr() as *mut libc::c_void, max_bytes) };

        if n_read > 0 {
            buf.truncate(n_read as usize);
            let bytes_ptr = alloc_bytes(&buf);
            if bytes_ptr == 0 {
                return alloc_result_box(1, error_text("falha na alocação"));
            }
            return alloc_result_box(0, bytes_ptr);
        }

        if n_read == 0 {
            // EOF.
            return alloc_result_box(1, error_text("EOF"));
        }

        // n_read < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
            // Sem dados — suspender fiber com o handle em socket_handles
            // para o scheduler fazer poll(POLLIN) no FD do socket.
            let suspended = crate::fiber::with_suspend(|suspend| {
                suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                    channel_handles: Vec::new(),
                    file_handles: Vec::new(),
                    socket_handles: vec![handle],
                    deadline: None,
                });
            });
            if suspended.is_none() {
                return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
            }
            // Fiber resumido — tentar novamente.
            continue;
        }

        // Erro real.
        return alloc_result_box(1, error_text(&format!("erro de leitura: {err}")));
    }
}

/// Escreve Text (C string) no socket.
///
/// Non-blocking: se o buffer de escrita está cheio (EAGAIN), suspende o fiber.
/// Loop até escrever todos os bytes.
///
/// # Safety
/// `handle` deve ser um handle válido.
/// `data_ptr` deve ser um ponteiro Text (C string) válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_write_text(handle: i64, data_ptr: i64) -> i64 {
    let inner = match socket_from_handle(handle) {
        Some(s) => s,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if inner.state != SocketState::Connected {
        return alloc_result_box(1, error_text("socket listener não suporta write"));
    }

    if data_ptr == 0 {
        return alloc_result_box(0, 0); // Ok(Unit) — nothing to write
    }

    let data = unsafe { CStr::from_ptr(data_ptr as *const c_char) };
    let bytes = data.to_bytes();

    if bytes.is_empty() {
        return alloc_result_box(0, 0);
    }

    write_all(inner, handle, bytes)
}

/// Escreve Bytes (blob com header de len) no socket.
///
/// # Safety
/// `handle` deve ser um handle válido.
/// `data_ptr` deve ser um ponteiro Bytes válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_write_bytes(handle: i64, data_ptr: i64) -> i64 {
    let inner = match socket_from_handle(handle) {
        Some(s) => s,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if inner.state != SocketState::Connected {
        return alloc_result_box(1, error_text("socket listener não suporta write"));
    }

    if data_ptr == 0 {
        return alloc_result_box(0, 0);
    }

    let len = unsafe { std::ptr::read_unaligned(data_ptr as *const i64) };
    if len <= 0 {
        return alloc_result_box(0, 0);
    }

    let data_slice =
        unsafe { std::slice::from_raw_parts((data_ptr as *const u8).add(8), len as usize) };

    write_all(inner, handle, data_slice)
}

/// Loop de escrita non-blocking com suspensão cooperativa.
/// Escreve até todos os bytes serem enviados.
/// `handle` é o ponteiro do SocketInner — necessário para suspender o fiber
/// com o handle em socket_handles (scheduler poll).
fn write_all(inner: &mut SocketInner, handle: i64, data: &[u8]) -> i64 {
    let mut written = 0usize;

    while written < data.len() {
        let n_written = unsafe {
            libc::write(
                inner.fd,
                data[written..].as_ptr() as *const libc::c_void,
                data.len() - written,
            )
        };

        if n_written > 0 {
            written += n_written as usize;
            continue;
        }

        if n_written == 0 {
            // Não deveria acontecer em write, mas tratamos.
            return alloc_result_box(1, error_text("write retornou 0"));
        }

        // n_written < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
            // Buffer cheio — suspender fiber com o handle em socket_handles
            // para o scheduler fazer poll(POLLIN | POLLOUT) no FD do socket.
            // POLLOUT é essencial: sem ele, o fiber nunca seria acordado quando
            // o socket ficasse gravável novamente.
            let suspended = crate::fiber::with_suspend(|suspend| {
                suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                    channel_handles: Vec::new(),
                    file_handles: Vec::new(),
                    socket_handles: vec![handle],
                    deadline: None,
                });
            });
            if suspended.is_none() {
                return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
            }
            // Fiber resumido — tentar novamente.
            continue;
        }

        // Erro real (EPIPE, ECONNRESET, etc).
        return alloc_result_box(1, error_text(&format!("erro de escrita: {err}")));
    }

    alloc_result_box(0, 0) // Ok(Unit)
}

/// Fecha o socket e marca como fechado.
///
/// Idempotente: se chamado múltiplas vezes (ex: close explícito + epílogo),
/// o campo `closed` garante que o FD só é fechado uma vez.
///
/// # Safety
/// `handle` deve ser um handle válido (ou 0 — no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_close(handle: i64) {
    if handle == 0 {
        return;
    }
    let inner = unsafe { &mut *(handle as *mut SocketInner) };
    if inner.closed {
        return;
    }
    inner.closed = true;
    unsafe {
        libc::close(inner.fd);
    }
}

// ── Select para Socket handles ─────────────────────────────────────

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

    let mut fds: Vec<libc::pollfd> = Vec::with_capacity(handles.len());
    let mut valid_indices: Vec<usize> = Vec::with_capacity(handles.len());

    for (idx, &handle) in handles.iter().enumerate() {
        if handle == 0 {
            continue;
        }
        let inner = unsafe { &*(handle as *const SocketInner) };
        if inner.closed {
            continue;
        }
        fds.push(libc::pollfd {
            fd: inner.fd,
            events: libc::POLLIN | libc::POLLOUT,
            revents: 0,
        });
        valid_indices.push(idx);
    }

    if fds.is_empty() {
        return SOCKET_WOULD_BLOCK;
    }

    let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 0) };
    if ret <= 0 {
        return SOCKET_WOULD_BLOCK;
    }

    for (i, pfd) in fds.iter().enumerate() {
        if pfd.revents & (libc::POLLIN | libc::POLLOUT | libc::POLLHUP) != 0 {
            return valid_indices[i] as i64;
        }
    }

    SOCKET_WOULD_BLOCK
}

/// Coleta os FDs brutos de socket handles válidos e não-fechados.
/// Usado pelo scheduler no sleep path para poll unificado.
/// Inclui POLLIN | POLLOUT — sockets podem bloquear tanto em read quanto write.
pub(crate) fn collect_socket_fds(handles: &[i64]) -> Vec<libc::pollfd> {
    let mut fds: Vec<libc::pollfd> = Vec::with_capacity(handles.len());

    for &handle in handles {
        if handle == 0 {
            continue;
        }
        let inner = unsafe { &*(handle as *const SocketInner) };
        if inner.closed {
            continue;
        }
        fds.push(libc::pollfd {
            fd: inner.fd,
            events: libc::POLLIN | libc::POLLOUT,
            revents: 0,
        });
    }

    fds
}
