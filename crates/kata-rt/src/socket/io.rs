//! I/O de sockets — read, read_chunk, readline, write_text, write_bytes, close.
//!
//! Non-blocking com suspensão cooperativa: EAGAIN/EWOULDBLOCK suspende o fiber
//! via `WaitingOnSelect` para o scheduler fazer poll no FD.

use super::{
    SocketInner, SocketState, alloc_bytes, alloc_result_box, alloc_text, error_text,
    socket_from_handle,
};
use crate::platform::{close_fd, is_would_block, raw_read, raw_write};
use std::ffi::CStr;
use std::os::raw::c_char;

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
        let n_read = raw_read(inner.fd, buf.as_mut_ptr(), buf.len());
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
        if is_would_block(err) {
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
        let n_read = raw_read(inner.fd, buf.as_mut_ptr(), max_bytes);

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
        if is_would_block(err) {
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

/// Lê uma linha do socket como Text (até `\n`).
///
/// Usa `line_buf` persistente em `SocketInner` para acumular bytes parciais
/// entre chamadas — TCP não preserva fronteiras de mensagem, então uma linha
/// pode chegar em múltiplos chunks. O buffer sobrevive entre chamadas.
///
/// Non-blocking: se não há dados (EAGAIN) e o buffer não tem `\n`, suspende
/// o fiber. EOF (read retorna 0): se o buffer tem dados, retorna como linha
/// parcial (sem `\n`); se vazio, retorna `Err("EOF")`.
///
/// Não misturar com `read`/`read_chunk` no mesmo socket — estas lêem do FD
/// diretamente, ignorando `line_buf`, e consomem bytes que readline esperava.
///
/// Retorna Result box Ok(text_ptr) ou Err(text_ptr).
///
/// # Safety
/// `handle` deve ser um handle válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_socket_readline(handle: i64) -> i64 {
    let inner = match socket_from_handle(handle) {
        Some(s) => s,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if inner.state != SocketState::Connected {
        return alloc_result_box(1, error_text("socket listener não suporta readline"));
    }

    let mut buf = [0u8; 8192];

    loop {
        // Verifica se já temos uma linha completa no buffer.
        if let Some(pos) = inner.line_buf.iter().position(|&b| b == b'\n') {
            // Extrai a linha (sem o \n).
            let line: Vec<u8> = inner.line_buf.drain(..=pos).collect();
            let line = &line[..line.len() - 1]; // remove \n
            // Remove \r final se presente (CRLF → LF).
            let line = if line.ends_with(b"\r") {
                &line[..line.len() - 1]
            } else {
                line
            };
            let text_ptr = alloc_text(std::str::from_utf8(line).unwrap_or(""));
            if text_ptr == 0 {
                return alloc_result_box(1, error_text("falha na alocação"));
            }
            return alloc_result_box(0, text_ptr);
        }

        // Tenta ler mais dados do socket.
        let n_read = raw_read(inner.fd, buf.as_mut_ptr(), buf.len());

        if n_read > 0 {
            inner.line_buf.extend_from_slice(&buf[..n_read as usize]);
            continue; // Re-check buffer para \n.
        }

        if n_read == 0 {
            // EOF — peer fechou. Se buffer tem dados, retorna como linha parcial.
            if !inner.line_buf.is_empty() {
                let line = std::mem::take(&mut inner.line_buf);
                // Remove \r final se presente.
                let line = if line.ends_with(b"\r") {
                    &line[..line.len() - 1]
                } else {
                    &line
                };
                let text_ptr = alloc_text(std::str::from_utf8(line).unwrap_or(""));
                if text_ptr == 0 {
                    return alloc_result_box(1, error_text("falha na alocação"));
                }
                return alloc_result_box(0, text_ptr);
            }
            return alloc_result_box(1, error_text("EOF"));
        }

        // n_read < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if is_would_block(err) {
            // Sem dados e sem linha completa — suspender fiber.
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
        let n_written = raw_write(inner.fd, data[written..].as_ptr(), data.len() - written);

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
        if is_would_block(err) {
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
    close_fd(inner.fd);
}
