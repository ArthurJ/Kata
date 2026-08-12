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
pub const POLLIN: i16 = 0x001;
pub const POLLOUT: i16 = 0x004;
pub const POLLHUP: i16 = 0x010;

/// Poll descriptor — layout compatível com `pollfd` e `WSAPOLLFD`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

// ── Bindings Win32 (Winsock2) ───────────────────────────────────────
#[cfg(windows)]
mod winsock {
    use std::ffi::c_int;

    pub const SOL_SOCKET: c_int = 0xffff;
    pub const SO_REUSEADDR: c_int = 0x0004;
    pub const FIONBIO: u32 = 0x8004667c;
    pub const WSAEWOULDBLOCK: c_int = 10035;

    pub const SD_RECEIVE: c_int = 0;
    pub const SD_SEND: c_int = 1;
    pub const SD_BOTH: c_int = 2;

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
    }
}

// ── set_nonblocking ────────────────────────────────────────────────

/// Configura FD como non-blocking.
#[cfg(unix)]
pub fn set_nonblocking(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

#[cfg(windows)]
pub fn set_nonblocking(fd: i32) {
    unsafe {
        let mut mode: i32 = 1;
        winsock::ioctlsocket(fd as usize, winsock::FIONBIO, &mut mode);
    }
}

// ── close_fd ────────────────────────────────────────────────────────

/// Fecha um FD/socket.
#[cfg(unix)]
pub fn close_fd(fd: i32) {
    unsafe {
        libc::close(fd);
    }
}

#[cfg(windows)]
pub fn close_fd(fd: i32) {
    unsafe {
        winsock::closesocket(fd as usize);
    }
}

// ── raw_read / raw_write ────────────────────────────────────────────
//
// No Unix, `read`/`write` funcionam para files, pipes e sockets.
// No Windows, sockets usam `recv`/`send`; files/pipes usam `ReadFile`/`WriteFile`.
// Para sockets (o caso principal no runtime), usamos `recv`/`send` no Windows.

/// Lê bytes de um FD/socket. Retorna número de bytes lidos, 0 para EOF, <0 para erro.
#[cfg(unix)]
pub fn raw_read(fd: i32, buf: *mut u8, len: usize) -> isize {
    unsafe { libc::read(fd, buf as *mut libc::c_void, len) as isize }
}

#[cfg(windows)]
pub fn raw_read(fd: i32, buf: *mut u8, len: usize) -> isize {
    unsafe { winsock::recv(fd as usize, buf, len as i32, 0) as isize }
}

/// Escreve bytes em um FD/socket. Retorna número de bytes escritos, <0 para erro.
#[cfg(unix)]
pub fn raw_write(fd: i32, buf: *const u8, len: usize) -> isize {
    unsafe { libc::write(fd, buf as *const libc::c_void, len) as isize }
}

#[cfg(windows)]
pub fn raw_write(fd: i32, buf: *const u8, len: usize) -> isize {
    unsafe { winsock::send(fd as usize, buf, len as i32, 0) as isize }
}

// ── poll_fds ────────────────────────────────────────────────────────

/// Poll em múltiplos FDs. Retorna número de FDs prontos, 0 para timeout, <0 para erro.
#[cfg(unix)]
pub fn poll_fds(fds: &mut [PollFd], timeout_ms: i32) -> i32 {
    unsafe {
        libc::poll(
            fds.as_mut_ptr() as *mut libc::pollfd,
            fds.len() as libc::nfds_t,
            timeout_ms,
        )
    }
}

#[cfg(windows)]
pub fn poll_fds(fds: &mut [PollFd], timeout_ms: i32) -> i32 {
    unsafe { winsock::WSAPoll(fds.as_mut_ptr(), fds.len() as u32, timeout_ms) }
}

// ── set_reuseaddr ───────────────────────────────────────────────────

/// Habilita SO_REUSEADDR no socket.
#[cfg(unix)]
pub fn set_reuseaddr(fd: i32) {
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
pub fn set_reuseaddr(fd: i32) {
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
pub fn tcp_listener_fd(listener: &std::net::TcpListener) -> i32 {
    use std::os::unix::io::AsRawFd;
    listener.as_raw_fd()
}

#[cfg(unix)]
pub fn tcp_listener_into_fd(listener: std::net::TcpListener) -> i32 {
    use std::os::unix::io::IntoRawFd;
    listener.into_raw_fd()
}

#[cfg(unix)]
pub fn tcp_stream_fd(stream: &std::net::TcpStream) -> i32 {
    use std::os::unix::io::AsRawFd;
    stream.as_raw_fd()
}

#[cfg(unix)]
pub fn tcp_stream_into_fd(stream: std::net::TcpStream) -> i32 {
    use std::os::unix::io::IntoRawFd;
    stream.into_raw_fd()
}

#[cfg(windows)]
pub fn tcp_listener_fd(listener: &std::net::TcpListener) -> i32 {
    use std::os::windows::io::AsRawSocket;
    listener.as_raw_socket() as i32
}

#[cfg(windows)]
pub fn tcp_listener_into_fd(listener: std::net::TcpListener) -> i32 {
    use std::os::windows::io::IntoRawSocket;
    listener.into_raw_socket() as i32
}

#[cfg(windows)]
pub fn tcp_stream_fd(stream: &std::net::TcpStream) -> i32 {
    use std::os::windows::io::AsRawSocket;
    stream.as_raw_socket() as i32
}

#[cfg(windows)]
pub fn tcp_stream_into_fd(stream: std::net::TcpStream) -> i32 {
    use std::os::windows::io::IntoRawSocket;
    stream.into_raw_socket() as i32
}

// ── raw_fd de File ──────────────────────────────────────────────────
//
// Extrai o FD/handle bruto de std::fs::File para poll. No Unix usa
// `as_raw_fd`; no Windows usa `as_raw_socket`.

#[cfg(unix)]
pub fn file_raw_fd(file: &std::fs::File) -> i32 {
    use std::os::unix::io::AsRawFd;
    file.as_raw_fd()
}

#[cfg(windows)]
pub fn file_raw_fd(file: &std::fs::File) -> i32 {
    use std::os::windows::io::AsRawHandle;
    file.as_raw_handle() as i32
}

// ── EAGAIN / EWOULDBLOCK ────────────────────────────────────────────

/// Verifica se o erro é "would block" (non-blocking, tentar novamente).
#[cfg(unix)]
pub fn is_would_block(errno: i32) -> bool {
    errno == libc::EAGAIN || errno == libc::EWOULDBLOCK
}

#[cfg(windows)]
pub fn is_would_block(errno: i32) -> bool {
    errno == winsock::WSAEWOULDBLOCK
}