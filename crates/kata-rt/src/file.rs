//! File I/O — handle opaco para arquivos abertos.
//!
//! Layout:
//! - `FileInner` alocado via `kata_rt_arena_alloc` na root_arena (sem header ARC).
//!   Handle = ponteiro puro para FileInner. O close faz `close_fd` no FD
//!   bruto. O campo `closed` no FileInner garante idempotência —
//!   o epílogo pode chamar close num handle já fechado sem double-free.
//! - FD bruto (`i32`) armazenado diretamente em `FileInner` — sem `BufReader`.
//!   Isto permite `raw_read`/`raw_write` direto no FD, non-blocking via
//!   `fcntl(O_NONBLOCK)`, e poll uniforme (igual `SocketInner`).
//! - `line_buf` persistente em `FileInner` para `readline` — acumula bytes
//!   parciais entre chamadas, igual `SocketInner`.
//! - Result boxes alocados via `kata_rt_arena_alloc` na root_arena (sem
//!   header ARC, sem destructor encadeado).
//! - Bytes de `read`/`read_chunk` alocados via `kata_rt_arena_alloc` na root_arena.
//! - Text de `readline` alocado via `kata_rt_arena_alloc` na root_arena.
//!
//! I/O cooperativo: reads em chunks de 64KB com yield cooperativo entre
//! syscalls. Se `raw_read` retorna EAGAIN (pipe/FIFO non-blocking), suspende
//! o fiber via `WaitingOnSelect`. O scheduler faz poll no FD e resume quando
//! há dados.
//!
//! FFI:
//! - `kata_rt_file_open(path_ptr, mode_tag, arena_handle) -> result_box` — Result::(File, Text)
//! - `kata_rt_file_read(handle) -> result_box` — Result::(Bytes, Text)
//! - `kata_rt_file_read_chunk(handle, n) -> result_box` — Result::(Bytes, Text)
//! - `kata_rt_file_readline(handle) -> result_box` — Result::(Text, Text)
//! - `kata_rt_file_write_text(handle, data_ptr) -> result_box` — Result::(Unit, Text)
//! - `kata_rt_file_write_bytes(handle, data_ptr) -> result_box` — Result::(Unit, Text)
//! - `kata_rt_file_close(handle) -> ()` — fecha arquivo (idempotente)
//!
//! Submódulos:
//! - `select`: seleção de file descriptors (helpers do scheduler —
//!   `try_select_files`, `collect_file_fds`, `kata_rt_select_files`).

use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::os::raw::c_char;

use crate::platform::{
    close_file_handle, file_into_raw_fd, is_would_block, raw_read_file, raw_write_file,
    set_nonblocking_file,
};

// ── Submódulos ─────────────────────────────────────────────────────
mod select;
mod stdio;
pub use select::kata_rt_select_files;
pub(crate) use select::{FILE_WOULD_BLOCK, collect_file_fds, try_select_files};
pub(crate) use stdio::reset_stdio_cache;
pub use stdio::{kata_rt_input, kata_rt_stderr, kata_rt_stdin, kata_rt_stdout};

// ── Constantes ────────────────────────────────────────────────────

/// Tamanho do chunk para leitura cooperativa (64KB).
const READ_CHUNK_SIZE: usize = 64 * 1024;

// ── IoHandle — camada comum para File e Socket ─────────────────────

/// Handle de I/O genérico — base para File e Socket.
/// `mode` indica quais operações são permitidas.
pub(crate) struct IoHandle {
    pub mode: IoMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoMode {
    Read,
    Write,
    Append,
    ReadWrite,
    Create,
}

/// Mapeia o tag do enum FileMode (0-4) para IoMode.
fn mode_from_tag(tag: i64) -> Option<IoMode> {
    match tag {
        0 => Some(IoMode::Read),
        1 => Some(IoMode::Write),
        2 => Some(IoMode::Append),
        3 => Some(IoMode::ReadWrite),
        4 => Some(IoMode::Create),
        _ => None,
    }
}

/// FileInner — arquivo aberto com FD bruto, line_buf e modo.
/// Alocado via `arena_alloc` na root_arena. O campo `closed` garante
/// que `kata_rt_file_close` é idempotente — múltiplas chamadas de close
/// (explícita + epílogo) não causam double-free.
///
/// O FD bruto permite `raw_read`/`raw_write` direto, non-blocking via
/// `fcntl(O_NONBLOCK)`, e poll uniforme com sockets. O `line_buf`
/// acumula bytes parciais para `readline` entre chamadas — mesmo
/// pattern do `SocketInner`.
///
/// Não misturar `readline` com `read`/`read_chunk` no mesmo handle:
/// `read`/`read_chunk` lêem do FD diretamente, ignorando `line_buf`,
/// e consomem bytes que `readline` esperava.
pub(crate) struct FileInner {
    pub closed: bool,
    pub fd: i32,
    pub io: IoHandle,
    /// Se true, o handle é um descritor padrão (FD 0/1/2).
    /// `kata_rt_file_close` é no-op. `read`/`readline` em stdout/stderr
    /// retornam `Err("not readable")`. `write` em stdin retorna
    /// `Err("not writable")`.
    pub is_stdio: bool,
    #[allow(dead_code)]
    pub path: String,
    /// Buffer parcial para readline — acumula bytes até encontrar \n.
    /// Usado apenas por `kata_rt_file_readline`; `read`/`read_chunk`
    /// lêem do FD diretamente.
    pub line_buf: Vec<u8>,
}

// ── Helpers ────────────────────────────────────────────────────────

/// Aloca um bloco na arena fornecida via `kata_rt_arena_alloc`.
/// Usado por `kata_rt_file_open` que recebe `arena_handle` do codegen
/// (baseado em escape analysis). `arena_alloc` (root_arena) permanece
/// para Result boxes e Text — esses não precisam de escape analysis.
fn arena_alloc_in(rt: i64, arena_handle: i64, size: i64) -> i64 {
    crate::arena::kata_rt_arena_alloc(rt, arena_handle, size)
}

/// Aloca um bloco na fiber arena (Bump) com fallback para root_arena.
///
/// A fiber arena é resetada quando o fiber termina. Se sem fiber ativo,
/// usa root_arena (Bump). Para FileInner, o close faz `drop_in_place`
/// para fechar o FD; a memória permanece na arena até o reset/teardown.
fn fiber_alloc(size: i64) -> i64 {
    let rt = crate::arena::rt_ptr();
    if rt == 0 {
        return 0;
    }
    let fiber_arena = crate::scheduler::CURRENT_FIBER_ARENA
        .with(|c| c.get())
        .unwrap_or(0);
    if fiber_arena > 0 {
        return crate::arena::kata_rt_arena_alloc(rt, fiber_arena, size);
    }
    let root_arena = crate::arena::kata_rt_get_root_arena_handle(rt);
    crate::arena::kata_rt_arena_alloc(rt, root_arena, size)
}

/// Aloca um Result box com tag e payload.
/// Layout do data: tag (i64) no offset 0, payload (i64) no offset 8.
pub(crate) fn alloc_result_box(tag: i64, payload: i64) -> i64 {
    let data_ptr = fiber_alloc(16);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut i64, tag);
        std::ptr::write_unaligned((data_ptr as *mut u8).add(8) as *mut i64, payload);
    }
    data_ptr
}

/// Aloca um FileInner na arena fornecida e retorna o ponteiro (handle).
fn alloc_file_inner_in(rt: i64, arena_handle: i64, inner: FileInner) -> i64 {
    let size = std::mem::size_of::<FileInner>() as i64;
    let data_ptr = arena_alloc_in(rt, arena_handle, size);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut FileInner, inner);
    }
    data_ptr
}

/// Aloca um FileInner na root_arena e retorna o ponteiro (handle).
/// Mantido para compatibilidade (stdio handles).
pub(crate) fn alloc_file_inner(inner: FileInner) -> i64 {
    let size = std::mem::size_of::<FileInner>() as i64;
    let data_ptr = fiber_alloc(size);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut FileInner, inner);
    }
    data_ptr
}

/// Extrai `FileInner` de um handle (ponteiro puro).
///
/// Retorna `None` se o handle é 0 (nulo).
pub(crate) fn file_from_handle(handle: i64) -> Option<&'static mut FileInner> {
    if handle == 0 {
        return None;
    }
    // SAFETY: o handle foi criado por `alloc_file_inner`, que alocou via
    // `arena_alloc` na root_arena. O ponteiro é válido enquanto a
    // root_arena existir (toda a duração do processo).
    Some(unsafe { &mut *(handle as *mut FileInner) })
}

/// Cria um Text a partir de uma String.
/// Text é representado como C string (nulo-terminada).
pub(crate) fn alloc_text(s: &str) -> i64 {
    let data_size = s.len() as i64 + 1; // bytes + null terminator
    let data_ptr = fiber_alloc(data_size);
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
/// Layout do blob Bytes: len (i64) no offset 0, data[i] no offset 8+i.
fn alloc_bytes(data: &[u8]) -> i64 {
    let data_size = 8 + data.len() as i64; // 8 (len) + data
    let data_ptr = fiber_alloc(data_size);
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

// ── OPEN_FILES registry ───────────────────────────────────────────
// Registry global de handles FileInner abertos (não-stdio).
// Usado por `reset_file_registry` para fechar FDs vazados entre testes.
// stdio handles (is_stdio=true) NÃO são registrados.

use std::cell::RefCell;

thread_local! {
    static OPEN_FILES: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
}

/// Registra um handle em OPEN_FILES (se não for stdio).
fn register_file_handle(handle: i64) {
    if handle == 0 {
        return;
    }
    // Verifica se é stdio — não registra.
    if let Some(inner) = file_from_handle(handle)
        && inner.is_stdio
    {
        return;
    }
    OPEN_FILES.with(|r| r.borrow_mut().push(handle));
}

/// Remove um handle de OPEN_FILES (se presente).
fn unregister_file_handle(handle: i64) {
    OPEN_FILES.with(|r| r.borrow_mut().retain(|&h| h != handle));
}

/// Fecha todos os FDs abertos não-stdio e limpa o registry.
/// Chamada por `reset_scheduler` entre testes.
pub(crate) fn reset_file_registry() {
    reset_stdio_cache();
    OPEN_FILES.with(|r| {
        let handles: Vec<i64> = r.borrow().iter().copied().collect();
        for handle in handles {
            // Fecha cada handle — kata_rt_file_close é idempotente.
            unsafe { kata_rt_file_close(handle) };
        }
        r.borrow_mut().clear();
    });
}

// ── FFI ────────────────────────────────────────────────────────────

/// Abre um arquivo e retorna um Result box.
///
/// `path_ptr` é um ponteiro C string (Text).
/// `mode_box` é um ponteiro para um Sum box (FileMode variant) — o tag
/// da variante (0=Read, 1=Write, etc.) é extraído via `sum_tag_int`.
///
/// Retorna:
/// - Result box Ok(handle) se sucesso — handle é ponteiro para FileInner.
/// - Result box Err(text) se erro — text é ponteiro para C string.
///
/// `kata_rt_file_open(path_ptr, mode_box, arena_handle) -> i64`
///
/// `arena_handle` é injetado pelo codegen via escape analysis:
/// - `Local` → `fiber_arena` (arquivo local à action/fiber)
/// - `Caller` → `caller_arena` (arquivo retornado pela action)
/// - `Heap` → `root_arena` (arquivo enviado via canal entre fibers)
///
/// # Safety
/// `path_ptr` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
/// `mode_box` deve ser um ponteiro válido para um Sum box (FileMode variant).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_open(
    path_ptr: *const c_char,
    mode_box: i64,
    arena_handle: i64,
) -> i64 {
    if path_ptr.is_null() {
        return alloc_result_box(1, error_text("path nulo"));
    }
    // Extrai o tag da variante FileMode do box.
    let mode_tag = if mode_box == 0 {
        0 // fallback: Read
    } else {
        crate::sum::kata_rt_sum_tag_int(mode_box)
    };

    // SAFETY: caller (JIT codegen) garante ponteiro C string válido.
    let path_cstr = unsafe { CStr::from_ptr(path_ptr) };
    let path = match path_cstr.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return alloc_result_box(1, error_text("path inválido (não UTF-8)")),
    };

    let mode = match mode_from_tag(mode_tag) {
        Some(m) => m,
        None => return alloc_result_box(1, error_text("modo inválido")),
    };

    let file = match mode {
        IoMode::Read => File::open(&path),
        IoMode::Write => File::create(&path),
        IoMode::Append => OpenOptions::new().append(true).create(true).open(&path),
        IoMode::ReadWrite => OpenOptions::new().read(true).write(true).open(&path),
        IoMode::Create => OpenOptions::new().write(true).create_new(true).open(&path),
    };

    let file = match file {
        Ok(f) => f,
        Err(e) => return alloc_result_box(1, error_text(&format!("erro ao abrir: {e}"))),
    };

    // Extrai o FD bruto do File e seta non-blocking.
    // Para arquivos regulares, O_NONBLOCK é no-op (kernel ignora).
    // Para pipes/FIFOs/FUSE, habilita EAGAIN — permite suspensão cooperativa.
    let fd = file_into_raw_fd(file);
    set_nonblocking_file(fd);

    let inner = FileInner {
        closed: false,
        fd,
        io: IoHandle { mode },
        is_stdio: false,
        path,
        line_buf: Vec::new(),
    };
    let handle = alloc_file_inner_in(crate::arena::rt_ptr(), arena_handle, inner);
    if handle == 0 {
        return alloc_result_box(1, error_text("falha na alocação"));
    }

    // Registra handle para cleanup entre testes.
    // Se o arquivo foi alocado na fiber_arena, registra em
    // FIBER_OPEN_FILES (fechado em try_destroy). Senão, registra em
    // OPEN_FILES (global, fechado em reset_file_registry).
    let fiber_arena = crate::scheduler::CURRENT_FIBER_ARENA.with(|c| c.get());
    if fiber_arena == Some(arena_handle) {
        // Arquivo fiber-local — registrar no TLS do fiber.
        crate::scheduler::FIBER_OPEN_FILES.with(|r| r.borrow_mut().push(handle));
    } else {
        // Arquivo global (root_arena ou caller_arena) — registrar no global.
        register_file_handle(handle);
    }

    // Ok box: tag=0, payload=handle.
    alloc_result_box(0, handle)
}

/// Helper: yield cooperativo entre chunks.
///
/// Suspende o fiber com `YieldReason::Cooperative` se há um fiber em
/// execução. Fora de fiber (teste unitário), é no-op.
fn yield_cooperative() {
    crate::fiber::with_suspend(|suspend| {
        suspend.suspend(crate::fiber::YieldReason::Cooperative);
    });
}

/// Helper: suspende o fiber esperando dados no FD (EAGAIN).
///
/// Suspende com `WaitingOnSelect` para o scheduler fazer poll no FD.
/// Fora de fiber, retorna `false` (não pode suspender).
fn suspend_waiting_on_file(handle: i64) -> bool {
    let suspended = crate::fiber::with_suspend(|suspend| {
        suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
            channel_handles: Vec::new(),
            file_handles: vec![handle],
            socket_handles: Vec::new(),
            deadline: None,
        });
    });
    suspended.is_some()
}

/// Helper: verifica se o modo permite leitura.
/// Retorna `Err` com mensagem apropriada se não permite.
fn check_read_mode(inner: &FileInner) -> Result<(), i64> {
    match inner.io.mode {
        IoMode::Read | IoMode::ReadWrite => Ok(()),
        _ => {
            let msg = if inner.is_stdio {
                "not readable"
            } else {
                "modo não permite leitura"
            };
            Err(error_text(msg))
        }
    }
}

/// Helper: verifica se o modo permite escrita.
/// Retorna `Err` com mensagem apropriada se não permite.
fn check_write_mode(inner: &FileInner) -> Result<(), i64> {
    match inner.io.mode {
        IoMode::Write | IoMode::Append | IoMode::ReadWrite | IoMode::Create => Ok(()),
        _ => {
            let msg = if inner.is_stdio {
                "not writable"
            } else {
                "modo não permite escrita"
            };
            Err(error_text(msg))
        }
    }
}

/// Lê todo o conteúdo do arquivo como Bytes.
///
/// I/O cooperativo: lê em chunks de 64KB com yield cooperativo entre
/// syscalls. Se `raw_read` retorna EAGAIN (pipe/FIFO non-blocking),
/// suspende o fiber via `WaitingOnSelect`.
///
/// Retorna Result box Ok(bytes_ptr) ou Err(text).
///
/// # Safety
/// `handle` deve ser um handle válido (criado por `kata_rt_file_open`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_read(handle: i64) -> i64 {
    let inner = match file_from_handle(handle) {
        Some(f) => f,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if let Err(e) = check_read_mode(inner) {
        return alloc_result_box(1, e);
    }

    let mut data = Vec::new();
    let mut buf = [0u8; READ_CHUNK_SIZE];

    // Drena line_buf primeiro — bytes que readline bufferizou de uma
    // chamada anterior devem ser visíveis a read/read_chunk.
    if !inner.line_buf.is_empty() {
        data.append(&mut inner.line_buf);
        yield_cooperative();
    }

    loop {
        let n_read = raw_read_file(inner.fd, buf.as_mut_ptr(), buf.len());

        if n_read > 0 {
            data.extend_from_slice(&buf[..n_read as usize]);
            // Yield cooperativo entre chunks — outros fibers rodam.
            yield_cooperative();
            continue;
        }

        if n_read == 0 {
            // EOF.
            break;
        }

        // n_read < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if is_would_block(err) {
            if data.is_empty() {
                // Sem dados — suspender fiber esperando poll no FD.
                if !suspend_waiting_on_file(handle) {
                    return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
                }
                // Fiber resumido — tentar novamente.
                continue;
            }
            // Já tem dados — retorna partial read.
            break;
        }

        // Erro real.
        return alloc_result_box(1, error_text(&format!("erro de leitura: {err}")));
    }

    if data.is_empty() {
        // EOF — tag 2 = variante Eof do ReadResult, sem payload.
        return alloc_result_box(2, 0);
    }

    let bytes_ptr = alloc_bytes(&data);
    if bytes_ptr == 0 {
        return alloc_result_box(1, error_text("falha na alocação"));
    }

    alloc_result_box(0, bytes_ptr)
}

/// Lê até `n` bytes do arquivo como Bytes.
///
/// `n` é um valor Int SMI-tagged (payload = n >> 1).
///
/// I/O cooperativo: lê em chunks de 64KB com yield cooperativo entre
/// syscalls. Se `raw_read` retorna EAGAIN (pipe/FIFO non-blocking),
/// suspende o fiber via `WaitingOnSelect`.
///
/// Retorna:
/// - Result box Ok(bytes_ptr) — bytes lidos (0 a n bytes).
/// - Result box Err("EOF") — quando 0 bytes lidos (fim do arquivo).
///
/// EOF como Err é consistente com `readline` — Err para EOF, Ok para dados.
///
/// # Safety
/// `handle` deve ser um handle válido (criado por `kata_rt_file_open`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_read_chunk(handle: i64, n: i64) -> i64 {
    let inner = match file_from_handle(handle) {
        Some(f) => f,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if let Err(e) = check_read_mode(inner) {
        return alloc_result_box(1, e);
    }

    // Decodifica SMI: n >> 1.
    let max_bytes = (n >> 1) as usize;
    let mut buf = vec![0u8; max_bytes];
    let mut total_read = 0usize;

    // Drena line_buf primeiro — bytes que readline bufferizou de uma
    // chamada anterior devem ser visíveis a read/read_chunk.
    if !inner.line_buf.is_empty() {
        let take = inner.line_buf.len().min(max_bytes);
        buf[..take].copy_from_slice(&inner.line_buf[..take]);
        total_read = take;
        inner.line_buf.drain(..take);
        if total_read >= max_bytes {
            // line_buf já tinha dados suficientes.
            buf.truncate(total_read);
            let bytes_ptr = alloc_bytes(&buf);
            if bytes_ptr == 0 {
                return alloc_result_box(1, error_text("falha na alocação"));
            }
            return alloc_result_box(0, bytes_ptr);
        }
        yield_cooperative();
    }

    while total_read < max_bytes {
        let chunk_end = (total_read + READ_CHUNK_SIZE).min(max_bytes);
        let n_read = raw_read_file(
            inner.fd,
            buf[total_read..].as_mut_ptr(),
            chunk_end - total_read,
        );

        if n_read > 0 {
            total_read += n_read as usize;
            // Yield cooperativo entre chunks.
            yield_cooperative();
            continue;
        }

        if n_read == 0 {
            // EOF.
            break;
        }

        // n_read < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if is_would_block(err) {
            if total_read == 0 {
                // Sem dados — suspender fiber.
                if !suspend_waiting_on_file(handle) {
                    return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
                }
                continue;
            }
            // Já tem dados — retorna partial read.
            break;
        }

        // Erro real.
        return alloc_result_box(1, error_text(&format!("erro de leitura: {err}")));
    }

    if total_read == 0 {
        // EOF — tag 2 = variante Eof do ReadResult, sem payload.
        return alloc_result_box(2, 0);
    }

    buf.truncate(total_read);
    let bytes_ptr = alloc_bytes(&buf);
    if bytes_ptr == 0 {
        return alloc_result_box(1, error_text("falha na alocação"));
    }

    alloc_result_box(0, bytes_ptr)
}

/// Lê uma linha do arquivo como Text (até `\n`).
///
/// Usa `line_buf` persistente em `FileInner` para acumular bytes parciais
/// entre chamadas. Non-blocking: se não há dados (EAGAIN) e o buffer não
/// tem `\n`, suspende o fiber. EOF (read retorna 0): se o buffer tem dados,
/// retorna como linha parcial (sem `\n`); se vazio, retorna `Err("EOF")`.
///
/// Não misturar com `read`/`read_chunk` no mesmo handle — estas lêem do FD
/// diretamente, ignorando `line_buf`, e consomem bytes que readline esperava.
///
/// Retorna Result box Ok(text_ptr) ou Err(text).
///
/// # Safety
/// `handle` deve ser um handle válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_readline(handle: i64) -> i64 {
    let inner = match file_from_handle(handle) {
        Some(f) => f,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if let Err(e) = check_read_mode(inner) {
        return alloc_result_box(1, e);
    }

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
            let text_ptr = alloc_text(std::str::from_utf8(line).unwrap_or(""));
            if text_ptr == 0 {
                return alloc_result_box(1, error_text("falha na alocação"));
            }
            return alloc_result_box(0, text_ptr);
        }

        let n_read = raw_read_file(inner.fd, buf.as_mut_ptr(), buf.len());

        if n_read > 0 {
            inner.line_buf.extend_from_slice(&buf[..n_read as usize]);
            // Yield cooperativo entre reads.
            yield_cooperative();
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
                let text_ptr = alloc_text(std::str::from_utf8(line).unwrap_or(""));
                if text_ptr == 0 {
                    return alloc_result_box(1, error_text("falha na alocação"));
                }
                return alloc_result_box(0, text_ptr);
            }
            return alloc_result_box(2, 0); // EOF — tag 2 = Eof
        }

        // n_read < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if is_would_block(err) {
            // Sem dados e sem linha completa — suspender fiber.
            if !suspend_waiting_on_file(handle) {
                return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
            }
            // Fiber resumido — tentar novamente.
            continue;
        }

        // Erro real.
        return alloc_result_box(1, error_text(&format!("erro de leitura: {err}")));
    }
}

/// Escreve Text (C string) no arquivo.
///
/// Usa `raw_write` direto no FD. Se o FD é non-blocking e o buffer está
/// cheio (EAGAIN), suspende o fiber via `WaitingOnSelect`.
///
/// `data_ptr` é um ponteiro Text (C string nulo-terminada).
///
/// Retorna Result box Ok(0) ou Err(text).
///
/// # Safety
/// `handle` deve ser um handle válido.
/// `data_ptr` deve ser um ponteiro Text (C string) válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_write_text(handle: i64, data_ptr: i64) -> i64 {
    let inner = match file_from_handle(handle) {
        Some(f) => f,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if let Err(e) = check_write_mode(inner) {
        return alloc_result_box(1, e);
    }

    if data_ptr == 0 {
        return alloc_result_box(0, 0); // Ok(Unit) — nothing to write
    }

    let data = unsafe { CStr::from_ptr(data_ptr as *const c_char) };
    let bytes = data.to_bytes();

    if bytes.is_empty() {
        return alloc_result_box(0, 0);
    }

    write_all_fd(inner, handle, bytes)
}

/// Escreve Bytes (blob com header de len) no arquivo.
///
/// `data_ptr` é um ponteiro Bytes (layout: len@0, data@8).
///
/// Retorna Result box Ok(0) ou Err(text).
///
/// # Safety
/// `handle` deve ser um handle válido.
/// `data_ptr` deve ser um ponteiro Bytes válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_write_bytes(handle: i64, data_ptr: i64) -> i64 {
    let inner = match file_from_handle(handle) {
        Some(f) => f,
        None => return alloc_result_box(1, error_text("handle inválido")),
    };

    if let Err(e) = check_write_mode(inner) {
        return alloc_result_box(1, e);
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

    write_all_fd(inner, handle, data_slice)
}

/// Loop de escrita com `raw_write` e suspensão cooperativa em EAGAIN.
///
/// Escreve até todos os bytes serem enviados. Se o FD é non-blocking e o
/// buffer está cheio (EAGAIN), suspende o fiber via `WaitingOnSelect`.
fn write_all_fd(inner: &mut FileInner, handle: i64, data: &[u8]) -> i64 {
    let mut written = 0usize;

    while written < data.len() {
        let n_written = raw_write_file(inner.fd, data[written..].as_ptr(), data.len() - written);

        if n_written > 0 {
            written += n_written as usize;
            continue;
        }

        if n_written == 0 {
            return alloc_result_box(1, error_text("write retornou 0"));
        }

        // n_written < 0 — erro.
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if is_would_block(err) {
            // Buffer cheio — suspender fiber.
            let suspended = crate::fiber::with_suspend(|suspend| {
                suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                    channel_handles: Vec::new(),
                    file_handles: vec![handle],
                    socket_handles: Vec::new(),
                    deadline: None,
                });
            });
            if suspended.is_none() {
                return alloc_result_box(1, error_text("WOULDBLOCK sem fiber"));
            }
            // Fiber resumido — tentar novamente.
            continue;
        }

        // Erro real (EPIPE, etc).
        return alloc_result_box(1, error_text(&format!("erro de escrita: {err}")));
    }

    alloc_result_box(0, 0) // Ok(Unit)
}

/// Fecha o arquivo via `close_fd` no FD bruto.
///
/// Idempotente: se chamado múltiplas vezes (ex: close explícito + epílogo),
/// o campo `closed` no FileInner garante que o FD só é fechado uma vez.
/// A memória do FileInner permanece na arena até o teardown do processo.
///
/// # Safety
/// `handle` deve ser um handle válido (ou 0 — no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_close(handle: i64) {
    if handle == 0 {
        return;
    }
    let inner = unsafe { &mut *(handle as *mut FileInner) };
    // stdio (FD 0/1/2) nunca fecha — is_stdio previne double-free.
    if inner.is_stdio || inner.closed {
        // Já fechado ou stdio — no-op (idempotente).
        return;
    }
    inner.closed = true;
    // Remove do registry antes de close (evita dangling no OPEN_FILES/FIBER_OPEN_FILES).
    unregister_file_handle(handle);
    crate::scheduler::FIBER_OPEN_FILES.with(|r| r.borrow_mut().retain(|&h| h != handle));
    // Fecha o FD via syscall direta. A memória do FileInner permanece na
    // arena até o teardown.
    close_file_handle(inner.fd);
}
