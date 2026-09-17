//! Helpers de plataforma — funções (não trait) com `#[cfg]` por OS.
//!
//! Cada função tem uma implementação POSIX (Linux + macOS) e uma Windows.
//! O resto do crate usa estas funções em vez de chamar `libc::` diretamente,
//! para que o código compile e funcione em ambos os platforms sem trait
//! overhead.

// ── PollFd comum ────────────────────────────────────────────────────
//
// `pollfd` (POSIX) e `WSAPOLLFD` (Windows) têm o mesmo layout:
// `{ fd: i32, events: i16, revents: i16 }`. Definimos um tipo comum
// para evitar `#[cfg]` em cada site de uso.

/// Eventos de poll — valores idênticos em POSIX e Winsock.
pub(crate) const POLLIN: i16 = 0x001;
pub(crate) const POLLOUT: i16 = 0x004;
pub(crate) const POLLHUP: i16 = 0x010;

/// Poll descriptor — layout compatível com `pollfd` e `WSAPOLLFD`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct PollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

// ── Bindings Win32 (Winsock2) ───────────────────────────────────────
#[cfg(windows)]
pub(crate) mod winsock {
    use std::ffi::c_int;

    pub const SOL_SOCKET: c_int = 0xffff;
    pub const SO_REUSEADDR: c_int = 0x0004;
    pub const FIONBIO: u32 = 0x8004667c;
    pub const WSAEWOULDBLOCK: c_int = 10035;
    pub const AF_INET: c_int = 2;
    #[allow(dead_code)]
    pub const AF_INET6: c_int = 23;
    pub const SOCK_STREAM: c_int = 1;
    pub const SOMAXCONN: c_int = 0x7fffffff;

    #[allow(dead_code)]
    pub const SD_RECEIVE: c_int = 0;
    #[allow(dead_code)]
    pub const SD_SEND: c_int = 1;
    #[allow(dead_code)]
    pub const SD_BOTH: c_int = 2;

    // sockaddr_in layout (IPv4)
    #[repr(C)]
    pub struct SockaddrIn {
        pub sin_family: u16,
        pub sin_port: u16,
        pub sin_addr: u32,
        pub sin_zero: [u8; 8],
    }

    // sockaddr layout (genérico)
    #[repr(C)]
    pub struct Sockaddr {
        pub sa_family: u16,
        pub sa_data: [u8; 14],
    }

    #[link(name = "ws2_32")]
    unsafe extern "C" {
        pub fn ioctlsocket(fd: usize, cmd: u32, argp: *mut c_int) -> c_int;
        pub fn closesocket(fd: usize) -> c_int;
        pub fn recv(fd: usize, buf: *mut u8, len: c_int, flags: c_int) -> c_int;
        pub fn send(fd: usize, buf: *const u8, len: c_int, flags: c_int) -> c_int;
        pub fn WSAPoll(fds: *mut super::PollFd, nfds: u32, timeout: c_int) -> c_int;
        pub fn setsockopt(
            fd: usize,
            level: c_int,
            optname: c_int,
            optval: *const u8,
            optlen: c_int,
        ) -> c_int;
        pub fn socket(af: c_int, sock_type: c_int, protocol: c_int) -> usize;
        pub fn bind(fd: usize, addr: *const Sockaddr, addrlen: c_int) -> c_int;
        pub fn listen(fd: usize, backlog: c_int) -> c_int;
        pub fn accept(fd: usize, addr: *mut Sockaddr, addrlen: *mut c_int) -> usize;
        pub fn connect(fd: usize, addr: *const Sockaddr, addrlen: c_int) -> c_int;
        pub fn htons(val: u16) -> u16;
        pub fn htonl(val: u32) -> u32;
        pub fn getsockname(fd: usize, addr: *mut Sockaddr, addrlen: *mut c_int) -> c_int;
        pub fn WSAStartup(version: u16, data: *mut [u8; 408]) -> c_int;
    }
}

// ── Bindings Win32 (Console + File I/O) ─────────────────────────────
#[cfg(windows)]
pub(crate) mod win32 {
    use std::ffi::c_void;

    // Constantes GetStdHandle — valores negativos castados para DWORD (u32).
    pub const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6; // (DWORD)-10
    pub const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5; // (DWORD)-11
    pub const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4; // (DWORD)-12

    pub type Handle = *mut c_void;

    #[link(name = "kernel32")]
    unsafe extern "C" {
        pub fn GetStdHandle(n_std_handle: u32) -> Handle;
        pub fn ReadFile(
            handle: Handle,
            buf: *mut u8,
            len: u32,
            bytes_read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        pub fn WriteFile(
            handle: Handle,
            buf: *const u8,
            len: u32,
            bytes_written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        pub fn CloseHandle(handle: Handle) -> i32;
        pub fn GetLastError() -> u32;
    }
}

// ── WSAStartup (inicialização Winsock) ──────────────────────────────

#[cfg(windows)]
pub(crate) fn ensure_winsock_init() {
    use std::sync::Once;
    static WSA_INIT: Once = Once::new();
    WSA_INIT.call_once(|| unsafe {
        let mut data: [u8; 408] = [0; 408];
        winsock::WSAStartup(0x0202, &mut data);
    });
}

// ── set_nonblocking ────────────────────────────────────────────────

/// Configura FD como non-blocking.
#[cfg(unix)]
pub(crate) fn set_nonblocking(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

#[cfg(windows)]
pub(crate) fn set_nonblocking(fd: i32) {
    unsafe {
        let mut mode: i32 = 1;
        winsock::ioctlsocket(fd as usize, winsock::FIONBIO, &mut mode);
    }
}

// ── close_fd ────────────────────────────────────────────────────────

/// Fecha um FD/socket.
#[cfg(unix)]
pub(crate) fn close_fd(fd: i32) {
    unsafe {
        libc::close(fd);
    }
}

#[cfg(windows)]
pub(crate) fn close_fd(fd: i32) {
    unsafe {
        winsock::closesocket(fd as usize);
    }
}

// ── raw_read / raw_write ────────────────────────────────────────────
//
// No Unix, `read`/`write` funcionam para files, pipes e sockets.
// No Windows, sockets usam `recv`/`send`; files/pipes usam `ReadFile`/`WriteFile`.
// Para sockets (o caso principal no runtime), usamos `recv`/`send` no Windows.
// Para files, `raw_read_file`/`raw_write_file` despacham para `ReadFile`/`WriteFile`.

/// Lê bytes de um FD/socket. Retorna número de bytes lidos, 0 para EOF, <0 para erro.
#[cfg(unix)]
pub(crate) fn raw_read(fd: i32, buf: *mut u8, len: usize) -> isize {
    unsafe { libc::read(fd, buf as *mut libc::c_void, len) as isize }
}

#[cfg(windows)]
pub(crate) fn raw_read(fd: i32, buf: *mut u8, len: usize) -> isize {
    unsafe { winsock::recv(fd as usize, buf, len as i32, 0) as isize }
}

/// Escreve bytes em um FD/socket. Retorna número de bytes escritos, <0 para erro.
#[cfg(unix)]
pub(crate) fn raw_write(fd: i32, buf: *const u8, len: usize) -> isize {
    unsafe { libc::write(fd, buf as *const libc::c_void, len) as isize }
}

#[cfg(windows)]
pub(crate) fn raw_write(fd: i32, buf: *const u8, len: usize) -> isize {
    unsafe { winsock::send(fd as usize, buf, len as i32, 0) as isize }
}

// ── raw_read_file / raw_write_file ──────────────────────────────────
//
// Funções separadas para file handles no Windows.
// No Unix, delegam para `raw_read`/`raw_write` (FD unificado).
// No Windows, usam `ReadFile`/`WriteFile` do kernel32.

/// Lê bytes de um file handle. Retorna número de bytes lidos, 0 para EOF, <0 para erro.
#[cfg(unix)]
pub(crate) fn raw_read_file(fd: i32, buf: *mut u8, len: usize) -> isize {
    raw_read(fd, buf, len)
}

#[cfg(windows)]
pub(crate) fn raw_read_file(fd: i32, buf: *mut u8, len: usize) -> isize {
    let handle = fd as *mut c_void;
    let mut bytes_read: u32 = 0;
    let rc = unsafe {
        win32::ReadFile(
            handle,
            buf,
            len as u32,
            &mut bytes_read,
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        bytes_read as isize
    } else {
        // ReadFile retorna 0 em erro ou EOF. Distinguir via GetLastError:
        // ERROR_HANDLE_EOF (38) ou ERROR_BROKEN_PIPE (109) = EOF (0).
        // Outros = erro (-1).
        let err = unsafe { win32::GetLastError() };
        if err == 38 || err == 109 { 0 } else { -1 }
    }
}

/// Escreve bytes em um file handle. Retorna número de bytes escritos, <0 para erro.
#[cfg(unix)]
pub(crate) fn raw_write_file(fd: i32, buf: *const u8, len: usize) -> isize {
    raw_write(fd, buf, len)
}

#[cfg(windows)]
pub(crate) fn raw_write_file(fd: i32, buf: *const u8, len: usize) -> isize {
    let handle = fd as *mut c_void;
    let mut bytes_written: u32 = 0;
    let rc = unsafe {
        win32::WriteFile(
            handle,
            buf,
            len as u32,
            &mut bytes_written,
            std::ptr::null_mut(),
        )
    };
    if rc != 0 { bytes_written as isize } else { -1 }
}

/// Fecha um file handle.
#[cfg(unix)]
pub(crate) fn close_file_handle(fd: i32) {
    close_fd(fd);
}

#[cfg(windows)]
pub(crate) fn close_file_handle(fd: i32) {
    unsafe { win32::CloseHandle(fd as *mut c_void) };
}

/// Configura file handle como non-blocking.
#[cfg(unix)]
pub(crate) fn set_nonblocking_file(fd: i32) {
    set_nonblocking(fd);
}

#[cfg(windows)]
pub(crate) fn set_nonblocking_file(_fd: i32) {
    // No Windows, files regulares são sempre "prontos" para I/O (como no Unix
    // com O_NONBLOCK — o kernel ignora para regular files). Pipes podem usar
    // PIPE_NOWAIT via SetNamedPipeHandleState, mas isso é edge case.
    // No-op para regular files.
}

// ── poll_fds ────────────────────────────────────────────────────────

/// Poll em múltiplos FDs. Retorna número de FDs prontos, 0 para timeout, <0 para erro.
#[cfg(unix)]
pub(crate) fn poll_fds(fds: &mut [PollFd], timeout_ms: i32) -> i32 {
    unsafe {
        libc::poll(
            fds.as_mut_ptr() as *mut libc::pollfd,
            fds.len() as libc::nfds_t,
            timeout_ms,
        )
    }
}

#[cfg(windows)]
pub(crate) fn poll_fds(fds: &mut [PollFd], timeout_ms: i32) -> i32 {
    unsafe { winsock::WSAPoll(fds.as_mut_ptr(), fds.len() as u32, timeout_ms) }
}

// ── set_reuseaddr ───────────────────────────────────────────────────

/// Habilita SO_REUSEADDR no socket.
#[cfg(unix)]
pub(crate) fn set_reuseaddr(fd: i32) {
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

#[cfg(windows)]
pub(crate) fn set_reuseaddr(fd: i32) {
    let optval: i32 = 1;
    unsafe {
        winsock::setsockopt(
            fd as usize,
            winsock::SOL_SOCKET,
            winsock::SO_REUSEADDR,
            &optval as *const _ as *const u8,
            std::mem::size_of::<i32>() as i32,
        );
    }
}

// ── raw_handle de TcpListener/TcpStream ─────────────────────────────
//
// Extrai o FD/handle bruto de TcpListener/TcpStream para armazenar
// no SocketInner. No Unix usa `as_raw_fd`/`into_raw_fd`; no Windows
// usa `as_raw_socket`/`into_raw_socket`.

#[cfg(unix)]
pub(crate) fn tcp_listener_fd(listener: &std::net::TcpListener) -> i32 {
    use std::os::unix::io::AsRawFd;
    listener.as_raw_fd()
}

#[cfg(unix)]
pub(crate) fn tcp_listener_into_fd(listener: std::net::TcpListener) -> i32 {
    use std::os::unix::io::IntoRawFd;
    listener.into_raw_fd()
}

#[cfg(unix)]
#[allow(dead_code)]
pub(crate) fn tcp_stream_fd(stream: &std::net::TcpStream) -> i32 {
    use std::os::unix::io::AsRawFd;
    stream.as_raw_fd()
}

#[cfg(unix)]
#[allow(dead_code)]
pub(crate) fn tcp_stream_into_fd(stream: std::net::TcpStream) -> i32 {
    use std::os::unix::io::IntoRawFd;
    stream.into_raw_fd()
}

#[cfg(windows)]
#[allow(dead_code)]
pub(crate) fn tcp_listener_fd(listener: &std::net::TcpListener) -> i32 {
    use std::os::windows::io::AsRawSocket;
    listener.as_raw_socket() as i32
}

#[cfg(windows)]
#[allow(dead_code)]
pub(crate) fn tcp_listener_into_fd(listener: std::net::TcpListener) -> i32 {
    use std::os::windows::io::IntoRawSocket;
    listener.into_raw_socket() as i32
}

#[cfg(windows)]
#[allow(dead_code)]
pub(crate) fn tcp_stream_fd(stream: &std::net::TcpStream) -> i32 {
    use std::os::windows::io::AsRawSocket;
    stream.as_raw_socket() as i32
}

#[cfg(windows)]
#[allow(dead_code)]
pub(crate) fn tcp_stream_into_fd(stream: std::net::TcpStream) -> i32 {
    use std::os::windows::io::IntoRawSocket;
    stream.into_raw_socket() as i32
}

// ── raw_fd de File ──────────────────────────────────────────────────
//
// Extrai o FD/handle bruto de std::fs::File para armazenar no FileInner.
// `into_raw_fd` consome o File (não fecha o FD — close via `close_fd`).

#[cfg(unix)]
pub(crate) fn file_into_raw_fd(file: std::fs::File) -> i32 {
    use std::os::unix::io::IntoRawFd;
    file.into_raw_fd()
}

#[cfg(windows)]
pub(crate) fn file_into_raw_fd(file: std::fs::File) -> i32 {
    use std::os::windows::io::IntoRawHandle;
    file.into_raw_handle() as i32
}

// ── EAGAIN / EWOULDBLOCK ────────────────────────────────────────────

/// Verifica se o erro é "would block" (non-blocking, tentar novamente).
#[cfg(unix)]
pub(crate) fn is_would_block(errno: i32) -> bool {
    errno == libc::EAGAIN || errno == libc::EWOULDBLOCK
}

#[cfg(windows)]
pub(crate) fn is_would_block(errno: i32) -> bool {
    errno == winsock::WSAEWOULDBLOCK
}
