//! Criação de sockets Unix-domain — extraído de `create.rs`.
//!
//! Contém as implementações Unix (`#[cfg(unix)]`, usando `UnixListener`/
//! `UnixStream`) e Windows (`#[cfg(windows)]`, fallback via TCP localhost +
//! arquivo de coordenação) para `create_unix_listener` e
//! `create_unix_connected`. As variantes TCP permanecem em [`create`].

use crate::platform::close_fd;

#[cfg(unix)]
use super::set_nonblocking;
use super::{
    SocketInner, SocketKindRust, SocketState, alloc_result_box, alloc_socket_inner, error_text,
};
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, IntoRawFd};
#[cfg(unix)]
use std::os::unix::net::UnixListener;

/// Cria listener Unix domain socket: bind + listen, non-blocking.
#[cfg(unix)]
pub(crate) fn create_unix_listener(path: &str) -> i64 {
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
pub(crate) fn create_unix_connected(path: &str) -> i64 {
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

/// Windows: Unix domain sockets não existem. Substitui por TCP localhost
/// com coordenação via arquivo temporário — o listener escreve a porta
/// atribuída num arquivo no `path`, o connected lê esse arquivo para
/// descobrir onde conectar. O `SocketKindRust::Unix` é preservado no
/// `SocketInner` para que o `close` saiba que não deve fazer `unlink`.
#[cfg(windows)]
pub(crate) fn create_unix_listener(path: &str) -> i64 {
    use crate::platform::winsock;

    crate::platform::ensure_winsock_init();

    // Criar socket TCP IPv4 em porta efêmera.
    let fd = unsafe { winsock::socket(winsock::AF_INET, winsock::SOCK_STREAM, 0) };
    if fd == usize::MAX {
        return alloc_result_box(1, error_text("socket() falhou"));
    }

    super::set_reuseaddr(fd as i32);

    // Bind em 127.0.0.1:0 (porta efêmera).
    let sa = winsock::SockaddrIn {
        sin_family: winsock::AF_INET as u16,
        sin_port: unsafe { winsock::htons(0) },
        sin_addr: unsafe { winsock::htonl(0x7f00_0001) }, // 127.0.0.1
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

    // Listen.
    let rc = unsafe { winsock::listen(fd, winsock::SOMAXCONN) };
    if rc != 0 {
        unsafe { winsock::closesocket(fd) };
        return alloc_result_box(1, error_text("listen falhou"));
    }

    // Descobrir porta atribuída.
    let mut local: winsock::SockaddrIn = unsafe { std::mem::zeroed() };
    let mut local_len = std::mem::size_of::<winsock::SockaddrIn>() as i32;
    let rc = unsafe {
        winsock::getsockname(
            fd,
            &mut local as *mut _ as *mut winsock::Sockaddr,
            &mut local_len,
        )
    };
    if rc != 0 {
        unsafe { winsock::closesocket(fd) };
        return alloc_result_box(1, error_text("getsockname falhou"));
    }
    let port = u16::from_be(local.sin_port);

    // Escrever porta no arquivo de coordenação (mesmo path que Unix usaria).
    // Remove arquivo anterior para reiniciar servidor.
    let _ = std::fs::remove_file(path);
    if std::fs::write(path, port.to_string()).is_err() {
        unsafe { winsock::closesocket(fd) };
        return alloc_result_box(1, error_text("falha ao escrever arquivo de coordenação"));
    }

    // Non-blocking.
    super::set_nonblocking(fd as i32);

    let inner = SocketInner {
        closed: false,
        fd: fd as i32,
        state: SocketState::Listener,
        kind: SocketKindRust::Unix,
        addr: path.to_string(),
        line_buf: Vec::new(),
    };
    let handle = alloc_socket_inner(inner);
    if handle == 0 {
        close_fd(fd as i32);
        return alloc_result_box(1, error_text("falha na alocação"));
    }
    alloc_result_box(0, handle)
}

/// Windows: Unix domain socket conectado via TCP localhost.
///
/// Lê a porta do arquivo de coordenação escrito pelo listener, conecta
/// em `127.0.0.1:<porta>`. Tem retry cooperativo — o servidor pode ainda
/// não ter feito listen.
#[cfg(windows)]
pub(crate) fn create_unix_connected(path: &str) -> i64 {
    use crate::platform::winsock;

    crate::platform::ensure_winsock_init();

    // Ceder controle ao scheduler — o servidor pode ainda não ter escrito
    // o arquivo de coordenação.
    let _ = crate::fiber::with_suspend(|suspend| {
        suspend.suspend(crate::fiber::YieldReason::Sleep(
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        ));
    });

    let max_retries = 50;
    for _ in 0..max_retries {
        // Ler porta do arquivo de coordenação.
        let port = match std::fs::read_to_string(path) {
            Ok(s) => match s.trim().parse::<u16>() {
                Ok(p) => p,
                Err(_) => {
                    // Arquivo existe mas inválido. Suspende e tenta de novo.
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
                        return alloc_result_box(1, error_text("arquivo de coordenação inválido"));
                    }
                    continue;
                }
            },
            Err(_) => {
                // Arquivo ainda não existe — servidor não iniciou. Suspende.
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
                    return alloc_result_box(
                        1,
                        error_text("arquivo de coordenação não encontrado"),
                    );
                }
                continue;
            }
        };

        // Criar socket e conectar.
        let fd = unsafe { winsock::socket(winsock::AF_INET, winsock::SOCK_STREAM, 0) };
        if fd == usize::MAX {
            return alloc_result_box(1, error_text("socket() falhou"));
        }

        let sa = winsock::SockaddrIn {
            sin_family: winsock::AF_INET as u16,
            sin_port: unsafe { winsock::htons(port) },
            sin_addr: unsafe { winsock::htonl(0x7f00_0001) },
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
            // ECONNREFUSED — servidor pode não ter feito listen ainda.
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
                return alloc_result_box(1, error_text("connect falhou"));
            }
            continue;
        }

        // Non-blocking após connect.
        super::set_nonblocking(fd as i32);

        let inner = SocketInner {
            closed: false,
            fd: fd as i32,
            state: SocketState::Connected,
            kind: SocketKindRust::Unix,
            addr: path.to_string(),
            line_buf: Vec::new(),
        };
        let handle = alloc_socket_inner(inner);
        if handle == 0 {
            close_fd(fd as i32);
            return alloc_result_box(1, error_text("falha na alocação"));
        }
        return alloc_result_box(0, handle);
    }
    alloc_result_box(1, error_text("connect falhou: timeout após retries"))
}
