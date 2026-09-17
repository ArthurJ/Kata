//! stdio: stdin/stdout/stderr como File.
//!
//! FFIs que retornam handles `File` apontando para FDs 0, 1 e 2.
//! O handle é `is_stdio: true` — `close!` é no-op, read/write guards
//! distinguem "not readable" (stdout/stderr) e "not writable" (stdin).
//!
//! Cache TLS: o handle é criado uma única vez (lazy) e cached.
//! Múltiplas chamadas a `__stdout__` retornam o mesmo handle.
//! `reset_file_registry` limpa o cache entre testes.

use std::cell::Cell;

use super::{alloc_text, file_from_handle, FileInner, IoHandle, IoMode, alloc_file_inner, READ_CHUNK_SIZE};
use crate::platform::{is_would_block, raw_read, set_nonblocking};

thread_local! {
    static STDIN_HANDLE: Cell<i64> = const { Cell::new(0) };
    static STDOUT_HANDLE: Cell<i64> = const { Cell::new(0) };
    static STDERR_HANDLE: Cell<i64> = const { Cell::new(0) };
}

/// Cria ou retorna o handle cached para um descritor padrão.
/// `fd` é 0 (stdin), 1 (stdout) ou 2 (stderr).
/// `mode` é `IoMode::Read` para stdin, `IoMode::Write` para stdout/stderr.
/// `label` é usado como path no FileInner (apenas para debug).
fn get_or_create_stdio(fd: i32, mode: IoMode, label: &str, cache: &Cell<i64>) -> i64 {
    let cached = cache.get();
    if cached != 0 {
        return cached;
    }
    // stdio FDs são non-blocking para permitir suspensão cooperativa
    // quando stdin é um pipe. Para terminal interativo, read() blocking
    // é o comportamento esperado (não retorna EAGAIN).
    set_nonblocking(fd);

    let inner = FileInner {
        closed: false,
        fd,
        io: IoHandle { mode },
        is_stdio: true,
        path: label.to_string(),
        line_buf: Vec::new(),
    };
    let handle = alloc_file_inner(inner);
    if handle != 0 {
        cache.set(handle);
    }
    handle
}

/// `kata_rt_stdin() -> i64` — handle `File` apontando para FD 0 (stdin).
///
/// Read-only. Múltiplas chamadas retornam o mesmo handle (TLS cache).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_stdin() -> i64 {
    STDIN_HANDLE.with(|c| get_or_create_stdio(0, IoMode::Read, "<stdin>", c))
}

/// `kata_rt_stdout() -> i64` — handle `File` apontando para FD 1 (stdout).
///
/// Write-only. Múltiplas chamadas retornam o mesmo handle (TLS cache).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_stdout() -> i64 {
    STDOUT_HANDLE.with(|c| get_or_create_stdio(1, IoMode::Write, "<stdout>", c))
}

/// `kata_rt_stderr() -> i64` — handle `File` apontando para FD 2 (stderr).
///
/// Write-only. Múltiplas chamadas retornam o mesmo handle (TLS cache).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_stderr() -> i64 {
    STDERR_HANDLE.with(|c| get_or_create_stdio(2, IoMode::Write, "<stderr>", c))
}

/// Limpa o cache de handles stdio entre testes.
/// Chamada por `reset_file_registry` ou diretamente por
/// `reset_scheduler`.
pub(crate) fn reset_stdio_cache() {
    STDIN_HANDLE.with(|c| c.set(0));
    STDOUT_HANDLE.with(|c| c.set(0));
    STDERR_HANDLE.with(|c| c.set(0));
}

/// `kata_rt_input(prompt_ptr) -> i64` — imprime prompt, lê uma linha de stdin.
///
/// Combina `print(prompt)` + `readline(stdin)` num único FFI call.
/// Usa `raw_read` + `line_buf` (mesmo pattern do `kata_rt_file_readline`).
/// Retorna Text (C string ptr). Em EOF ou erro, retorna Text vazio ("").
///
/// # Safety
/// `prompt_ptr` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_input(prompt_ptr: *const std::os::raw::c_char) -> i64 {
    use std::io::Write;

    // Imprime o prompt (sem newline) em stdout.
    if !prompt_ptr.is_null() {
        let cstr = unsafe { std::ffi::CStr::from_ptr(prompt_ptr) };
        let prompt = cstr.to_string_lossy();
        print!("{prompt}");
        let _ = std::io::stdout().flush();
    }

    // Lê uma linha de stdin via raw_read + line_buf.
    let handle = kata_rt_stdin();
    let inner = match file_from_handle(handle) {
        Some(f) => f,
        None => return alloc_text(""),
    };

    let mut buf = [0u8; READ_CHUNK_SIZE];

    loop {
        // Verifica se já temos uma linha completa no buffer.
        if let Some(pos) = inner.line_buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = inner.line_buf.drain(..=pos).collect();
            let line = &line[..line.len() - 1]; // remove \n
            let line = if line.ends_with(b"\r") {
                &line[..line.len() - 1]
            } else {
                line
            };
            return alloc_text(std::str::from_utf8(line).unwrap_or(""));
        }

        let n_read = raw_read(inner.fd, buf.as_mut_ptr(), buf.len());

        if n_read > 0 {
            inner.line_buf.extend_from_slice(&buf[..n_read as usize]);
            continue;
        }

        if n_read == 0 {
            // EOF — se buffer tem dados, retorna como linha parcial.
            if !inner.line_buf.is_empty() {
                let line = std::mem::take(&mut inner.line_buf);
                let line = if line.ends_with(b"\r") {
                    &line[..line.len() - 1]
                } else {
                    &line
                };
                return alloc_text(std::str::from_utf8(line).unwrap_or(""));
            }
            return alloc_text("");
        }

        // n_read < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if is_would_block(err) {
            // stdin non-blocking sem dados — retorna vazio (não bloqueia).
            // Em terminal interativo, read() é blocking e não chega aqui.
            // Em pipe sem dados, retornar vazio é mais seguro que suspender
            // (kata_rt_input não tem acesso ao mecanismo de suspensão).
            if !inner.line_buf.is_empty() {
                let line = std::mem::take(&mut inner.line_buf);
                let line = if line.ends_with(b"\r") {
                    &line[..line.len() - 1]
                } else {
                    &line
                };
                return alloc_text(std::str::from_utf8(line).unwrap_or(""));
            }
            return alloc_text("");
        }

        // Erro real.
        return alloc_text("");
    }
}