//! File I/O — handle opaco para arquivos abertos.
//!
//! Layout:
//! - `FileInner` alocado via `kata_rt_arena_alloc` na root_arena (sem header ARC).
//!   Handle = ponteiro puro para FileInner. O close faz `drop_in_place`
//!   (fecha o FD). O campo `closed` no FileInner garante idempotência —
//!   o epílogo pode chamar close num handle já fechado sem double-free.
//! - `BufReader<File>` persistente dentro de `FileInner` — todos os reads
//!   (read, read_chunk, readline) passam pelo mesmo BufReader. Isto resolve
//!   o bug de readline (recriar BufReader perde bytes bufferizados em
//!   arquivos > 8KB) e previne state corruption entre read_chunk e
//!   readline intercalados.
//! - Result boxes alocados via `kata_rt_arena_alloc` na root_arena (sem
//!   header ARC, sem destructor encadeado).
//! - Bytes de `read`/`read_chunk` alocados via `kata_rt_arena_alloc` na root_arena.
//! - Text de `readline` alocado via `kata_rt_arena_alloc` na root_arena.
//!
//! FFI:
//! - `kata_rt_file_open(path_ptr, mode_tag) -> result_box` — Result::(File, Text)
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
use std::io::{BufRead, BufReader, Read, Write};
use std::os::raw::c_char;

// ── Submódulos ─────────────────────────────────────────────────────
mod select;
pub use select::kata_rt_select_files;
pub(crate) use select::{FILE_WOULD_BLOCK, collect_file_fds, try_select_files};

// ── IoHandle — camada comum para File e futuro Socket ──────────────

/// Handle de I/O genérico — base para File e Socket.
/// `mode` indica quais operações são permitidas.
/// O `File` (descritor OS) vive dentro do `BufReader` em `FileInner`.
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

/// FileInner — arquivo aberto com path, BufReader persistente e modo.
/// Alocado via `arena_alloc` na root_arena. O campo `closed` garante
/// que `kata_rt_file_close` é idempotente — múltiplas chamadas de close
/// (explícita + epílogo) não causam double-free.
///
/// O `BufReader<File>` persistente garante que todos os reads (read,
/// read_chunk, readline) compartilham o mesmo buffer interno. Isto resolve:
/// 1. Bug de readline que recriava BufReader a cada chamada (perdia bytes
///    bufferizados em arquivos > 8KB).
/// 2. State corruption entre read_chunk e readline intercalados (cursos
///    de leitura divergentes).
pub(crate) struct FileInner {
    pub closed: bool,
    pub buf_reader: BufReader<File>,
    pub io: IoHandle,
    #[allow(dead_code)]
    pub path: String,
}

// ── Helpers ────────────────────────────────────────────────────────

/// Aloca um bloco na root_arena via `kata_rt_arena_alloc`.
/// Sem header ARC — a memória é liberada quando a root_arena for destruída
/// (fim do processo). Para FileInner, o close faz `drop_in_place` para
/// fechar o FD; a memória permanece na arena até o teardown.
fn arena_alloc(size: i64) -> i64 {
    let rt = crate::arena::rt_ptr();
    let root_arena = crate::arena::kata_rt_get_root_arena_handle(rt);
    crate::arena::kata_rt_arena_alloc(rt, root_arena, size)
}

/// Aloca um Result box com tag e payload.
/// Layout do data: tag (i64) no offset 0, payload (i64) no offset 8.
fn alloc_result_box(tag: i64, payload: i64) -> i64 {
    let data_ptr = arena_alloc(16);
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(data_ptr as *mut i64, tag);
        std::ptr::write_unaligned((data_ptr as *mut u8).add(8) as *mut i64, payload);
    }
    data_ptr
}

/// Aloca um FileInner e retorna o ponteiro (handle).
fn alloc_file_inner(inner: FileInner) -> i64 {
    let size = std::mem::size_of::<FileInner>() as i64;
    let data_ptr = arena_alloc(size);
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
fn file_from_handle(handle: i64) -> Option<&'static mut FileInner> {
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
fn alloc_text(s: &str) -> i64 {
    let data_size = s.len() as i64 + 1; // bytes + null terminator
    let data_ptr = arena_alloc(data_size);
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
    let data_ptr = arena_alloc(data_size);
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
/// # Safety
/// `path_ptr` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
/// `mode_box` deve ser um ponteiro válido para um Sum box (FileMode variant).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_open(path_ptr: *const c_char, mode_box: i64) -> i64 {
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

    let inner = FileInner {
        closed: false,
        buf_reader: BufReader::new(file),
        io: IoHandle { mode },
        path,
    };
    let handle = alloc_file_inner(inner);
    if handle == 0 {
        return alloc_result_box(1, error_text("falha na alocação"));
    }

    // Ok box: tag=0, payload=handle.
    alloc_result_box(0, handle)
}

/// Lê todo o conteúdo do arquivo como Bytes.
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

    // Verifica que o modo permite leitura.
    match inner.io.mode {
        IoMode::Read | IoMode::ReadWrite => {}
        _ => return alloc_result_box(1, error_text("modo não permite leitura")),
    }

    let mut data = Vec::new();
    if inner.buf_reader.read_to_end(&mut data).is_err() {
        return alloc_result_box(1, error_text("erro de leitura"));
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

    // Verifica que o modo permite leitura.
    match inner.io.mode {
        IoMode::Read | IoMode::ReadWrite => {}
        _ => return alloc_result_box(1, error_text("modo não permite leitura")),
    }

    // Decodifica SMI: n >> 1.
    let max_bytes = (n >> 1) as usize;

    let mut buf = vec![0u8; max_bytes];
    let mut total_read = 0usize;

    // read_buf pode retornar menos bytes que solicitado (especialmente
    // com BufReader). Loop até preencher buf ou atingir EOF.
    while total_read < max_bytes {
        match inner.buf_reader.read(&mut buf[total_read..]) {
            Ok(0) => break, // EOF
            Ok(n) => total_read += n,
            Err(_) => return alloc_result_box(1, error_text("erro de leitura")),
        }
    }

    if total_read == 0 {
        // EOF — consistente com readline.
        return alloc_result_box(1, error_text("EOF"));
    }

    buf.truncate(total_read);
    let bytes_ptr = alloc_bytes(&buf);
    if bytes_ptr == 0 {
        return alloc_result_box(1, error_text("falha na alocação"));
    }

    alloc_result_box(0, bytes_ptr)
}

/// Lê uma linha do arquivo como Text.
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

    // Verifica que o modo permite leitura.
    match inner.io.mode {
        IoMode::Read | IoMode::ReadWrite => {}
        _ => return alloc_result_box(1, error_text("modo não permite leitura")),
    }

    // Usa o BufReader persistente — bytes bufferizados de read_chunk
    // ou readline anterior são preservados entre chamadas.
    let mut line = String::new();
    match inner.buf_reader.read_line(&mut line) {
        Ok(0) => return alloc_result_box(1, error_text("EOF")), // EOF → Err
        Ok(_) => {}
        Err(_) => return alloc_result_box(1, error_text("erro de leitura")),
    }

    // Remove o \n ou \r\n do final, se presente.
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }

    let text_ptr = alloc_text(&line);
    if text_ptr == 0 {
        return alloc_result_box(1, error_text("falha na alocação"));
    }

    alloc_result_box(0, text_ptr)
}

/// Escreve Text (C string) no arquivo.
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

    // Verifica que o modo permite escrita.
    match inner.io.mode {
        IoMode::Write | IoMode::Append | IoMode::ReadWrite | IoMode::Create => {}
        _ => return alloc_result_box(1, error_text("modo não permite escrita")),
    }

    if data_ptr == 0 {
        // Nothing to write — Ok(Unit).
        return alloc_result_box(0, 0);
    }

    // Text é C string — lê até o null terminator.
    let data = unsafe { CStr::from_ptr(data_ptr as *const c_char) };
    let bytes = data.to_bytes();

    // BufReader::get_mut() dá acesso ao File subjacente para escrita.
    let file = inner.buf_reader.get_mut();
    match file.write_all(bytes) {
        Ok(_) => alloc_result_box(0, 0), // Ok(Unit)
        Err(e) => alloc_result_box(1, error_text(&format!("erro de escrita: {e}"))),
    }
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

    // Verifica que o modo permite escrita.
    match inner.io.mode {
        IoMode::Write | IoMode::Append | IoMode::ReadWrite | IoMode::Create => {}
        _ => return alloc_result_box(1, error_text("modo não permite escrita")),
    }

    if data_ptr == 0 {
        // Nothing to write — Ok(Unit).
        return alloc_result_box(0, 0);
    }

    // Bytes tem header: len (i64) no offset 0, data no offset 8.
    let len = unsafe { std::ptr::read_unaligned(data_ptr as *const i64) };
    if len <= 0 {
        return alloc_result_box(0, 0); // Ok(Unit) — nothing to write
    }

    let data_slice =
        unsafe { std::slice::from_raw_parts((data_ptr as *const u8).add(8), len as usize) };

    // BufReader::get_mut() dá acesso ao File subjacente para escrita.
    let file = inner.buf_reader.get_mut();
    match file.write_all(data_slice) {
        Ok(_) => alloc_result_box(0, 0), // Ok(Unit)
        Err(e) => alloc_result_box(1, error_text(&format!("erro de escrita: {e}"))),
    }
}

/// Fecha o arquivo e libera o FileInner via drop_in_place.
///
/// Idempotente: se chamado múltiplas vezes (ex: close explícito + epílogo),
/// o campo `closed` no FileInner garante que o FD só é fechado uma vez.
/// O `drop_in_place` roda o Drop do FileInner (fecha FD via drop de
/// BufReader→File, libera String do path) sem chamar dealloc — a memória
/// permanece na root_arena até o teardown do processo.
///
/// # Safety
/// `handle` deve ser um handle válido (ou 0 — no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_file_close(handle: i64) {
    if handle == 0 {
        return;
    }
    let inner = unsafe { &mut *(handle as *mut FileInner) };
    if inner.closed {
        // Já fechado — no-op (idempotente).
        return;
    }
    inner.closed = true;
    // drop_in_place roda o Drop de FileInner (fecha BufReader→FD, libera String)
    // sem chamar dealloc — a memória será liberada quando a root_arena
    // for destruída no fim do processo.
    unsafe {
        std::ptr::drop_in_place(handle as *mut FileInner);
    }
}
