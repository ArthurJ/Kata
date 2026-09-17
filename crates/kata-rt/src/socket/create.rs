//! Criação de sockets — FFI `kata_rt_socket_open` + `kata_rt_socket_accept`.
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
use std::net::{SocketAddr, TcpListener};
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

/// Cria socket TCP conectado: non-blocking connect com suspensão cooperativa.
///
/// Usa o padrão BSD/POSIX de non-blocking connect:
/// 1. `socket()` + `fcntl(O_NONBLOCK)` — cria socket non-blocking
/// 2. `connect()` — retorna EINPROGRESS (ou erro imediato se ECONNREFUSED)
/// 3. Se EINPROGRESS: alocar SocketInner, suspender fiber com POLLOUT
/// 4. Scheduler faz poll por POLLOUT — acorda quando connect completa
/// 5. `getsockopt(SO_ERROR)` — verifica se conectou ou erro
///
/// **Alocação na fiber arena:** todos os Result boxes e textos de erro são
/// alocados na fiber arena (Bump) em vez da root_arena (Tracked). A
/// TrackedArena usa `std::alloc::alloc` que, quando chamada de dentro de
/// uma FFI executada pelo JIT, corrompe intermitentemente a heap do
/// processo. A Bump arena (bumpalo) não tem esse problema.
#[cfg(unix)]
fn create_tcp_connected(addr: &str) -> i64 {
    let sock_addr: SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(e) => return alloc_err_fiber(&format!("endereço inválido: {e}")),
    };

    // Ceder controle ao scheduler antes do primeiro connect — o servidor
    // (fork!) pode ainda não ter executado bind+listen.
    let _ = crate::fiber::with_suspend(|suspend| {
        suspend.suspend(crate::fiber::YieldReason::Sleep(
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        ));
    });

    let max_retries = 50;
    for _ in 0..max_retries {
        match tcp_connect_nonblocking(&sock_addr) {
            ConnectResult::Connected(fd) => {
                let inner = SocketInner {
                    closed: false,
                    fd,
                    state: SocketState::Connected,
                    kind: SocketKindRust::Tcp,
                    addr: sock_addr.to_string(),
                    line_buf: Vec::new(),
                };
                let handle = alloc_socket_inner_fiber(inner);
                if handle == 0 {
                    close_fd(fd);
                    return alloc_err_fiber("falha na alocação");
                }
                return alloc_ok_fiber(handle);
            }
            ConnectResult::Refused => {
                // ECONNREFUSED — servidor não está ouvindo. Suspende com Sleep
                // e tenta novamente (evita busy-wait).
                let suspended = crate::fiber::with_suspend(|suspend| {
                    suspend.suspend(crate::fiber::YieldReason::Sleep(
                        std::time::Instant::now() + std::time::Duration::from_millis(100),
                    ));
                });
                if suspended.is_none() {
                    return alloc_err_fiber("connect falhou: sem fiber");
                }
            }
            ConnectResult::Error(msg) => {
                return alloc_err_fiber(&msg);
            }
        }
    }
    alloc_err_fiber("connect falhou: timeout após retries")
}

/// Aloca um bloco na fiber arena (Bump) em vez da root_arena (Tracked).
///
/// A TrackedArena corrompe intermitentemente a heap quando chamada de
/// dentro de FFIs executadas pelo JIT. A Bump arena (bumpalo) é segura.
/// Se a fiber arena não estiver disponível (fora de fiber), fallback
/// para a root_arena.
#[cfg(unix)]
fn fiber_alloc(size: i64) -> i64 {
    let rt = crate::arena::rt_ptr();
    if rt == 0 {
        return 0;
    }
    // Tentar fiber arena primeiro (Bump — segura).
    let fiber_arena = crate::scheduler::CURRENT_FIBER_ARENA
        .with(|c| c.get())
        .unwrap_or(0);
    if fiber_arena > 0 {
        return crate::arena::kata_rt_arena_alloc(rt, fiber_arena, size);
    }
    // Fallback: root_arena (Tracked — pode corromper, mas só se sem fiber).
    let root_arena = crate::arena::kata_rt_get_root_arena_handle(rt);
    crate::arena::kata_rt_arena_alloc(rt, root_arena, size)
}

/// Aloca um texto (C string nulo-terminada) na fiber arena.
#[cfg(unix)]
fn alloc_text_fiber(msg: &str) -> i64 {
    let data_size = msg.len() as i64 + 1;
    let data_ptr = fiber_alloc(data_size);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(msg.as_ptr(), data_ptr as *mut u8, msg.len());
        std::ptr::write_unaligned((data_ptr as *mut u8).add(msg.len()), 0);
    }
    data_ptr
}

/// Aloca um Result box Ok(handle) na fiber arena.
#[cfg(unix)]
fn alloc_ok_fiber(handle: i64) -> i64 {
    let data_ptr = fiber_alloc(16);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut i64, 0);
        std::ptr::write_unaligned((data_ptr as *mut u8).add(8) as *mut i64, handle);
    }
    data_ptr
}

/// Aloca um Result box Err(text) na fiber arena.
#[cfg(unix)]
fn alloc_err_fiber(msg: &str) -> i64 {
    let text_ptr = alloc_text_fiber(msg);
    let data_ptr = fiber_alloc(16);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut i64, 1);
        std::ptr::write_unaligned((data_ptr as *mut u8).add(8) as *mut i64, text_ptr);
    }
    data_ptr
}

/// Aloca um SocketInner na fiber arena.
#[cfg(unix)]
fn alloc_socket_inner_fiber(inner: SocketInner) -> i64 {
    let size = std::mem::size_of::<SocketInner>() as i64;
    let data_ptr = fiber_alloc(size);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut SocketInner, inner);
    }
    data_ptr
}

/// Resultado de uma tentativa de non-blocking connect.
#[cfg(unix)]
enum ConnectResult {
    /// Connectado com sucesso. FD do socket (non-blocking, ownership transferida).
    Connected(i32),
    /// ECONNREFUSED — servidor não está ouvindo. Caller deve tentar de novo.
    Refused,
    /// Erro irrecoverável (socket creation, getsockopt, etc).
    Error(String),
}

/// Faz um non-blocking connect TCP.
///
/// Cria socket non-blocking, chama connect(), e se EINPROGRESS, suspende o
/// fiber esperando POLLOUT. Ao resumir, verifica SO_ERROR via getsockopt.
///
/// Retorna `Connected(fd)` se sucesso, `Refused` se ECONNREFUSED, ou
/// `Error(msg)` para erros irrecoveráveis.
#[cfg(unix)]
fn tcp_connect_nonblocking(sock_addr: &SocketAddr) -> ConnectResult {
    // 1. Criar socket TCP non-blocking.
    #[cfg(target_os = "linux")]
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_NONBLOCK, 0) };
    #[cfg(not(target_os = "linux"))]
    let fd = {
        let f = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        if f >= 0 {
            set_nonblocking(f);
        }
        f
    };

    if fd < 0 {
        return ConnectResult::Error(format!("socket() falhou: {}", std::io::Error::last_os_error()));
    }

    // 2. connect() non-blocking.
    let (sin_addr, sin_port) = match sock_addr {
        SocketAddr::V4(v4) => {
            let octets = v4.ip().octets();
            (
                libc::in_addr {
                    s_addr: u32::from_ne_bytes(octets),
                },
                v4.port(),
            )
        }
        SocketAddr::V6(_) => {
            close_fd(fd);
            return ConnectResult::Error("IPv6 não suportado".into());
        }
    };

    let mut addr_in: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr_in.sin_family = libc::AF_INET as u16;
    addr_in.sin_port = sin_port.to_be();
    addr_in.sin_addr = sin_addr;

    let rc = unsafe {
        libc::connect(
            fd,
            &addr_in as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };

    if rc == 0 {
        // Connect imediato (localhost às vezes conecta instantaneamente).
        return ConnectResult::Connected(fd);
    }

    let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);

    if err == libc::EINPROGRESS {
        // 3. Connect em andamento — esperar com poll blocking curto.
        // Usar poll diretamente (não suspende o fiber) para evitar
        // interferência com o scheduler.
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let _ = unsafe { libc::poll(&mut pfd, 1, 200) };

        // 4. Verificar SO_ERROR via getsockopt.
        let mut so_error: libc::c_int = 0;
        let mut opt_len: libc::socklen_t = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut so_error as *mut _ as *mut libc::c_void,
                &mut opt_len,
            )
        };

        if rc < 0 {
            // Erro no getsockopt — não devemos chegar aqui.
            close_fd(fd);
            return ConnectResult::Error(format!("getsockopt falhou: {}", std::io::Error::last_os_error()));
        }

        if so_error == 0 {
            // Conectado com sucesso. O handle já tem o SocketInner.
            return ConnectResult::Connected(fd);
        }

        // Erro de connect (ECONNREFUSED, timeout, etc).
        let err_msg = std::io::Error::from_raw_os_error(so_error).to_string();
        close_fd(fd);
        if so_error == libc::ECONNREFUSED {
            return ConnectResult::Refused;
        }
        return ConnectResult::Error(err_msg);
    }

    // Erro imediato (ECONNREFUSED em localhost é instantâneo).
    close_fd(fd);
    if err == libc::ECONNREFUSED {
        return ConnectResult::Refused;
    }
    ConnectResult::Error(format!("connect falhou: {}", std::io::Error::last_os_error()))
}

/// Aceita uma conexão no listener (non-blocking com suspensão cooperativa).
///
/// Retorna Result box Ok(connected_handle) ou Err(text).
///
/// # Safety
/// `listener_handle` deve ser um handle válido criado por `kata_rt_socket_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_accept(listener_handle: i64) -> i64 {
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
