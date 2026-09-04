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
use std::fs::File;
use std::io::{BufRead, BufReader, Write};

use super::{FileInner, IoHandle, IoMode, alloc_file_inner, alloc_text, file_from_handle};

thread_local! {
    static STDIN_HANDLE: Cell<i64> = const { Cell::new(0) };
    static STDOUT_HANDLE: Cell<i64> = const { Cell::new(0) };
    static STDERR_HANDLE: Cell<i64> = const { Cell::new(0) };
}

/// Cria ou retorna o handle cached para um descritor padrão.
/// `fd` é 0 (stdin), 1 (stdout) ou 2 (stderr).
/// `mode` é `IoMode::Read` para stdin, `IoMode::Write` para stdout/stderr.
/// `label` é usado como path no FileInner (apenas para debug).
#[cfg(unix)]
fn get_or_create_stdio(fd: i32, mode: IoMode, label: &str, cache: &Cell<i64>) -> i64 {
    use std::os::unix::io::FromRawFd;

    let cached = cache.get();
    if cached != 0 {
        return cached;
    }
    // SAFETY: `from_raw_fd` toma ownership do FD. Como `is_stdio` previne
    // `close` e `Bump::reset` não chama Drop, o FD nunca é fechado pelo
    // runtime — seguro na prática. O FD 0/1/2 pertence ao processo.
    let file = unsafe { File::from_raw_fd(fd) };
    alloc_stdio_inner(file, mode, label, cache)
}

/// Windows: stdin/stdout/stderr via GetStdHandle + FromRawHandle.
///
/// `GetStdHandle` retorna um HANDLE Win32 para o dispositivo de console
/// ou pipe/redirecionamento ativo. `File::from_raw_handle` envolve esse
/// HANDLE em `std::fs::File`, cujo I/O usa `ReadFile`/`WriteFile` (não
/// `recv`/`send` do Winsock).
#[cfg(windows)]
fn get_or_create_stdio(fd: i32, mode: IoMode, label: &str, cache: &Cell<i64>) -> i64 {
    use crate::platform::win32;
    use std::os::windows::io::FromRawHandle;

    let cached = cache.get();
    if cached != 0 {
        return cached;
    }

    let std_handle = match fd {
        0 => win32::STD_INPUT_HANDLE,
        1 => win32::STD_OUTPUT_HANDLE,
        2 => win32::STD_ERROR_HANDLE,
        _ => return 0,
    };
    let handle = unsafe { win32::GetStdHandle(std_handle) };
    if handle.is_null() || handle as usize == usize::MAX {
        return 0;
    }
    // SAFETY: GetStdHandle retorna um HANDLE válido pertencente ao processo.
    // Como `is_stdio` previne `close`, o handle nunca é fechado pelo runtime.
    let file = unsafe { File::from_raw_handle(handle as std::os::windows::io::RawHandle) };
    alloc_stdio_inner(file, mode, label, cache)
}

fn alloc_stdio_inner(file: File, mode: IoMode, label: &str, cache: &Cell<i64>) -> i64 {
    let inner = FileInner {
        closed: false,
        buf_reader: BufReader::new(file),
        io: IoHandle { mode },
        is_stdio: true,
        path: label.to_string(),
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
/// Retorna Text (C string ptr). Em EOF ou erro, retorna Text vazio ("").
///
/// # Safety
/// `prompt_ptr` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_input(prompt_ptr: *const std::os::raw::c_char) -> i64 {
    // Imprime o prompt (sem newline) em stdout.
    if !prompt_ptr.is_null() {
        let cstr = unsafe { std::ffi::CStr::from_ptr(prompt_ptr) };
        let prompt = cstr.to_string_lossy();
        print!("{prompt}");
        let _ = std::io::stdout().flush();
    }

    // Lê uma linha de stdin via BufReader do handle cached.
    let handle = kata_rt_stdin();
    let inner = match file_from_handle(handle) {
        Some(f) => f,
        None => return alloc_text(""),
    };

    let mut line = String::new();
    match inner.buf_reader.read_line(&mut line) {
        Ok(0) => return alloc_text(""), // EOF → Text vazio
        Ok(_) => {}
        Err(_) => return alloc_text(""), // erro → Text vazio
    }

    // Remove \n ou \r\n do final.
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }

    alloc_text(&line)
}
