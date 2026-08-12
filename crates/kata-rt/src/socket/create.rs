//! Criação de sockets — FFI `kata_rt_socket_open` + `kata_rt_socket_listen`.
//!
//! Cria listeners TCP/Unix e sockets conectados, com retry cooperativo
//! para conectados (o servidor fork pode não ter feito listen ainda).

use crate::platform::close_fd;

use super::{
    alloc_result_box, alloc_socket_inner, error_text, set_nonblocking, set_reuseaddr, SocketInner,
    SocketKindRust, SocketState,
};
use std::ffi::CStr;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::raw::c_char;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, IntoRawFd};
#[cfg(unix)]
use std::os::unix::net::UnixListener;

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
#[cfg(unix)]
fn create_tcp_listener(addr: &str) -> i64 {
    let sock_addr: SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(e) => return alloc_result_box(1, error_text(&format!("endereço inválido: {e}"))),
    };

    let listener = match TcpListener::bind(sock_addr) {
        Ok(l) => l,
        Err(e) => return alloc_result_box(1, error_text(&format!("bind falhou: {e}"))),
    };

    set_reuseaddr(crate::platform::tcp_listener_fd(&listener));
    set_nonblocking(crate::platform::tcp_listener_fd(&listener));

    let fd = crate::platform::tcp_listener_into_fd(listener);

    let inner = SocketInner {
        closed: false,
        fd,
        state: SocketState::Listener,
        kind: SocketKindRust::Tcp,
        addr: sock_addr.to_string(),
        line_buf: Vec::new(),
    };
    let handle = alloc_socket_inner(inner);
    if handle == 0 {
        close_fd(fd);
        return alloc_result_box(1, error_text("falha na alocação"));
    }
    alloc_result_box(0, handle)
}

/// Cria socket TCP conectado: connect blocking com timeout.
///
/// Usa `TcpStream::connect_timeout` do Rust std (blocking). Se o servidor não
/// estiver ouvindo, suspende o fiber e tenta novamente (o servidor pode ainda
/// não ter feito listen, especialmente em testes com fork!).
#[cfg(unix)]
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
                set_nonblocking(crate::platform::tcp_stream_fd(&stream));
                let fd = crate::platform::tcp_stream_into_fd(stream);
                let inner = SocketInner {
                    closed: false,
                    fd,
                    state: SocketState::Connected,
                    kind: SocketKindRust::Tcp,
                    addr: sock_addr.to_string(),
                    line_buf: Vec::new(),
                };
                let handle = alloc_socket_inner(inner);
                if handle == 0 {
                    close_fd(fd);
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
#[cfg(unix)]
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
        line_buf: Vec::new(),
    };
    let handle = alloc_socket_inner(inner);
    if handle == 0 {
        close_fd(fd);
        return alloc_result_box(1, error_text("falha na alocação"));
    }
    alloc_result_box(0, handle)
}

/// Cria socket Unix conectado: connect com retry cooperativo.
#[cfg(unix)]
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
                    line_buf: Vec::new(),
                };
                let handle = alloc_socket_inner(inner);
                if handle == 0 {
                    close_fd(fd);
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
    let inner = match super::socket_from_handle(listener_handle) {
        Some(s) => s,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if inner.state != SocketState::Listener {
        return alloc_result_box(1, error_text("socket conectado não aceita conexões"));
    }

    loop {
        // Accept non-blocking. Plataforma-específico.
        let client_fd = accept_nonblocking(inner.fd);

        if client_fd >= 0 {
            let client_inner = SocketInner {
                closed: false,
                fd: client_fd,
                state: SocketState::Connected,
                kind: inner.kind,
                addr: String::new(),
                line_buf: Vec::new(),
            };
            let handle = alloc_socket_inner(client_inner);
            if handle == 0 {
                close_fd(client_fd);
                return alloc_result_box(1, error_text("falha na alocação"));
            }
            return alloc_result_box(0, handle);
        }

        // Erro — verificar se é would-block (non-blocking, sem conexão pendente).
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if crate::platform::is_would_block(err) {
            // Sem conexão pendente — suspender fiber, scheduler poll.
            let suspended = crate::fiber::with_suspend(|suspend| {
                suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                    channel_handles: Vec::new(),
                    file_handles: Vec::new(),
                    socket_handles: vec![listener_handle],
                    deadline: None,
                });
            });
            if suspended.is_none() {
                return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
            }
            continue;
        }

        // Erro real de accept.
        return alloc_result_box(1, error_text(&format!("accept falhou: {err}")));
    }
}

/// Accept non-blocking — implementação POSIX.
#[cfg(unix)]
fn accept_nonblocking(fd: i32) -> i32 {
    // Linux: accept4 com SOCK_NONBLOCK (atômico).
    // macOS: accept + fcntl(F_SETFL, O_NONBLOCK) (accept4 não disponível).
    let mut client_addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut addr_len: libc::socklen_t =
        std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;

    #[cfg(target_os = "linux")]
    {
        unsafe {
            libc::accept4(
                fd,
                &mut client_addr as *mut _ as *mut libc::sockaddr,
                &mut addr_len,
                libc::SOCK_NONBLOCK,
            )
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        unsafe {
            let new_fd = libc::accept(
                fd,
                &mut client_addr as *mut _ as *mut libc::sockaddr,
                &mut addr_len,
            );
            if new_fd >= 0 {
                let flags = libc::fcntl(new_fd, libc::F_GETFL, 0);
                if flags >= 0 {
                    libc::fcntl(new_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
            }
            new_fd
        }
    }
}

/// Accept non-blocking — implementação Windows (Winsock).
#[cfg(windows)]
fn accept_nonblocking(fd: i32) -> i32 {
    // TODO: Implementar accept via Winsock. Ver PRD-Windows Fase 2/4.
    // Por ora, retorna -1 (erro) — accept não funciona em Windows ainda.
    let _ = fd;
    -1
}

// ── Stubs Windows para criação de sockets ──────────────────────────
//
// No Windows, a criação de sockets TCP/Unix precisa de Winsock. Por ora,
// retornamos erro — o crate compila mas sockets não funcionam em Windows.
// A implementação completa é Fase 2/4 do PRD-Windows.

#[cfg(windows)]
fn create_tcp_listener(_addr: &str) -> i64 {
    alloc_result_box(1, error_text("socket TCP listener não suportado em Windows (TODO)"))
}

#[cfg(windows)]
fn create_tcp_connected(_addr: &str) -> i64 {
    alloc_result_box(1, error_text("socket TCP connected não suportado em Windows (TODO)"))
}

#[cfg(windows)]
fn create_unix_listener(_path: &str) -> i64 {
    alloc_result_box(1, error_text("Unix domain sockets não existem em Windows"))
}

#[cfg(windows)]
fn create_unix_connected(_path: &str) -> i64 {
    alloc_result_box(1, error_text("Unix domain sockets não existem em Windows"))
}
