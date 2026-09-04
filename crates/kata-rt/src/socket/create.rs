//! Criação de sockets — FFI `kata_rt_socket_open` + `kata_rt_socket_listen`.
//!
//! Cria listeners TCP/Unix e sockets conectados, com retry cooperativo
//! para conectados (o servidor fork pode não ter feito listen ainda).

use crate::platform::close_fd;

use super::create_unix::{create_unix_connected, create_unix_listener};
use super::{
    SocketInner, SocketKindRust, SocketState, alloc_result_box, alloc_socket_inner, error_text,
};
#[cfg(unix)]
use super::{set_nonblocking, set_reuseaddr};
use std::ffi::CStr;
#[cfg(unix)]
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::raw::c_char;

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
    use crate::platform::winsock;

    let mut addr: winsock::Sockaddr = unsafe { std::mem::zeroed() };
    let mut addr_len: i32 = std::mem::size_of::<winsock::Sockaddr>() as i32;

    let client_fd = unsafe { winsock::accept(fd as usize, &mut addr, &mut addr_len) };

    if client_fd == usize::MAX {
        // Erro — caller verifica is_would_block.
        return -1;
    }

    // Non-blocking no novo socket.
    super::set_nonblocking(client_fd as i32);

    client_fd as i32
}

// ── Implementação Windows (Winsock) para TCP sockets ───────────────
//
// Unix domain sockets não existem em Windows (named pipes no lugar).
// TCP listener/connected implementados via Winsock2.

#[cfg(windows)]
fn create_tcp_listener(addr: &str) -> i64 {
    use crate::platform::winsock;

    crate::platform::ensure_winsock_init();

    let (ip, port) = match parse_addr(addr) {
        Some(v) => v,
        None => return alloc_result_box(1, error_text(&format!("endereço inválido: {addr}"))),
    };

    // Criar socket TCP IPv4.
    let fd = unsafe { winsock::socket(winsock::AF_INET, winsock::SOCK_STREAM, 0) };
    if fd == usize::MAX {
        return alloc_result_box(1, error_text("socket() falhou"));
    }

    // SO_REUSEADDR.
    super::set_reuseaddr(fd as i32);

    // bind.
    let sa = winsock::SockaddrIn {
        sin_family: winsock::AF_INET as u16,
        sin_port: unsafe { winsock::htons(port) },
        sin_addr: unsafe { winsock::htonl(ip) },
        sin_zero: [0; 8],
    };
    let rc = unsafe {
        winsock::bind(
            fd,
            &sa as *const _ as *const winsock::Sockaddr,
            std::mem::size_of::<winsock::SockaddrIn>() as i32,
        )
    };
    if rc != 0 {
        unsafe { winsock::closesocket(fd) };
        return alloc_result_box(1, error_text("bind falhou"));
    }

    // listen.
    let rc = unsafe { winsock::listen(fd, winsock::SOMAXCONN) };
    if rc != 0 {
        unsafe { winsock::closesocket(fd) };
        return alloc_result_box(1, error_text("listen falhou"));
    }

    // Non-blocking.
    super::set_nonblocking(fd as i32);

    let inner = SocketInner {
        closed: false,
        fd: fd as i32,
        state: SocketState::Listener,
        kind: SocketKindRust::Tcp,
        addr: addr.to_string(),
        line_buf: Vec::new(),
    };
    let handle = alloc_socket_inner(inner);
    if handle == 0 {
        close_fd(fd as i32);
        return alloc_result_box(1, error_text("falha na alocação"));
    }
    alloc_result_box(0, handle)
}

#[cfg(windows)]
fn create_tcp_connected(addr: &str) -> i64 {
    use crate::platform::winsock;

    crate::platform::ensure_winsock_init();

    let (ip, port) = match parse_addr(addr) {
        Some(v) => v,
        None => return alloc_result_box(1, error_text(&format!("endereço inválido: {addr}"))),
    };

    // Criar socket TCP IPv4.
    let fd = unsafe { winsock::socket(winsock::AF_INET, winsock::SOCK_STREAM, 0) };
    if fd == usize::MAX {
        return alloc_result_box(1, error_text("socket() falhou"));
    }

    // Connect blocking.
    let sa = winsock::SockaddrIn {
        sin_family: winsock::AF_INET as u16,
        sin_port: unsafe { winsock::htons(port) },
        sin_addr: unsafe { winsock::htonl(ip) },
        sin_zero: [0; 8],
    };
    let rc = unsafe {
        winsock::connect(
            fd,
            &sa as *const _ as *const winsock::Sockaddr,
            std::mem::size_of::<winsock::SockaddrIn>() as i32,
        )
    };
    if rc != 0 {
        unsafe { winsock::closesocket(fd) };
        return alloc_result_box(1, error_text("connect falhou"));
    }

    // Non-blocking após connect.
    super::set_nonblocking(fd as i32);

    let inner = SocketInner {
        closed: false,
        fd: fd as i32,
        state: SocketState::Connected,
        kind: SocketKindRust::Tcp,
        addr: addr.to_string(),
        line_buf: Vec::new(),
    };
    let handle = alloc_socket_inner(inner);
    if handle == 0 {
        close_fd(fd as i32);
        return alloc_result_box(1, error_text("falha na alocação"));
    }
    alloc_result_box(0, handle)
}

/// Parse "ip:port" → (ip as u32 in network byte order, port as u16).
#[cfg(windows)]
fn parse_addr(addr: &str) -> Option<(u32, u16)> {
    let parts: Vec<&str> = addr.rsplitn(2, ':').collect();
    if parts.len() != 2 {
        return None;
    }
    let port: u16 = parts[0].parse().ok()?;
    let ip_parts: Vec<&str> = parts[1].split('.').collect();
    if ip_parts.len() != 4 {
        return None;
    }
    let mut ip_bytes = [0u8; 4];
    for (i, part) in ip_parts.iter().enumerate() {
        ip_bytes[i] = part.parse().ok()?;
    }
    Some((u32::from_le_bytes(ip_bytes), port))
}
