//! Marshalling — `to_bytes` / `from_bytes` para `spawn!` e IPC cross-process.
//!
//! `to_bytes` serializa um valor runtime em um blob `Bytes` com header estendido
//! (type_id + rebase_offsets), reaproveitando a mecânica de `HeapSnapshotData`
//! (Fio 12). `from_bytes` reconstrói o valor na arena destino.
//!
//! ## Type table
//!
//! `TypeShape` é a projeção runtime de `Ty` — carrega apenas a informação
//! estrutural necessária para caminhar o valor (fields, variants, element
//! types). A type table é registrada Rust-to-Rust pelo driver (não via FFI
//! C-ABI) antes de executar o JIT, armazenada em TLS. As FFIs `kata_rt_to_bytes`
//! e `kata_rt_from_bytes` consultam a type table por `type_id`.
//!
//! ## Layout do blob
//!
//! ```text
//! Bytes header (8 bytes):
//!   offset 0:  content_len (i64) — tamanho do conteúdo (sem este header)
//!
//! Conteúdo (content_len bytes):
//!   offset 0:   data_len (i64) — tamanho dos dados serializados
//!   offset 8:   type_id (i64) — tipo do valor (índice na type table)
//!   offset 16:  rebase_count (i64) — número de offsets para rebasing
//!   offset 24:  rebase_offsets[rebase_count] (i64 cada)
//!   offset 24+rebase_count*8: data[0..data_len] — bytes serializados
//! ```

use std::cell::RefCell;

/// Projeção runtime de `Ty` para marshalling — informação estrutural mínima
/// para caminhar um valor em runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeShape {
    /// Int, Float, Byte — 8 bytes inline (SMI-tagged ou raw bits).
    Prim,
    /// Unit — zero bytes.
    Unit,
    /// Text — C string (ponteiro na arena, nulo-terminada).
    Text,
    /// Bytes — blob contíguo (ponteiro na arena, layout 8+len).
    Bytes,
    /// Tupla — elementos heterogêneos, cada um 8 bytes.
    Tuple(Vec<TypeShape>),
    /// Struct — campos em ordem de declaração, cada um 8 bytes.
    Struct(Vec<TypeShape>),
    /// Sum (enum) — variantes com payload opcional.
    /// tag (i64) no offset 0, payload (i64) no offset 8.
    Sum(Vec<Option<Box<TypeShape>>>),
    /// List — Cons cells (head: 8 bytes, tail: ptr|0).
    List(Box<TypeShape>),
    /// Array — contíguo (len: i64, elements: i64 cada).
    Array(Box<TypeShape>),
}

thread_local! {
    static TYPE_TABLE: RefCell<Vec<TypeShape>> = const { RefCell::new(Vec::new()) };
}

/// Registra a type table Rust-to-Rust. Chamado pelo driver antes do JIT.
pub fn register_type_table(types: Vec<TypeShape>) {
    TYPE_TABLE.with(|table| {
        *table.borrow_mut() = types;
    });
}

/// Reseta a type table — chamado entre execuções de teste.
pub(crate) fn reset_type_table() {
    TYPE_TABLE.with(|table| {
        table.borrow_mut().clear();
    });
}

fn get_type_shape(type_id: i64) -> Option<TypeShape> {
    TYPE_TABLE.with(|table| {
        let table = table.borrow();
        table.get(type_id as usize).cloned()
    })
}

// ════════════════════════════════════════════════════════════
//  Serialização (to_bytes)
// ════════════════════════════════════════════════════════════

/// Serializador — duas regiões (main + appended), igual ao comptime.
struct Serializer {
    main: Vec<u8>,
    appended: Vec<u8>,
    /// Offsets na main onde há ponteiros relativos para a própria main.
    rebase_offsets: Vec<usize>,
    /// Offsets na main onde há ponteiros para a appended.
    appended_rebase_offsets: Vec<usize>,
}

impl Serializer {
    fn new() -> Self {
        Serializer {
            main: Vec::new(),
            appended: Vec::new(),
            rebase_offsets: Vec::new(),
            appended_rebase_offsets: Vec::new(),
        }
    }

    fn align_main(&mut self) {
        while !self.main.len().is_multiple_of(8) {
            self.main.push(0);
        }
    }

    fn write_i64(&mut self, val: i64) {
        self.align_main();
        self.main.extend_from_slice(&val.to_le_bytes());
    }

    fn write_main_ptr(&mut self, main_offset: i64) {
        self.align_main();
        let pos = self.main.len();
        self.main.extend_from_slice(&main_offset.to_le_bytes());
        if main_offset != 0 {
            self.rebase_offsets.push(pos);
        }
    }

    fn write_appended_ptr(&mut self, appended_offset: usize) {
        self.align_main();
        let pos = self.main.len();
        self.main
            .extend_from_slice(&(appended_offset as i64).to_le_bytes());
        self.appended_rebase_offsets.push(pos);
    }

    fn write_nil(&mut self) {
        self.align_main();
        self.main.extend_from_slice(&0i64.to_le_bytes());
    }

    fn write_appended(&mut self, bytes: &[u8]) -> usize {
        let offset = self.appended.len();
        self.appended.extend_from_slice(bytes);
        offset
    }

    fn main_aligned_len(&self) -> usize {
        let len = self.main.len();
        if len.is_multiple_of(8) {
            len
        } else {
            len + (8 - len % 8)
        }
    }

    fn finish(mut self) -> (Vec<u8>, Vec<usize>) {
        let main_len = self.main.len() as i64;
        for &pos in &self.appended_rebase_offsets {
            let ptr = self.main.as_mut_ptr();
            unsafe {
                let val_ptr = ptr.add(pos) as *mut i64;
                let val = std::ptr::read_unaligned(val_ptr);
                std::ptr::write_unaligned(val_ptr, val + main_len);
            }
        }
        let mut all = self.rebase_offsets;
        all.extend_from_slice(&self.appended_rebase_offsets);
        let mut bytes = self.main;
        bytes.extend_from_slice(&self.appended);
        (bytes, all)
    }
}

unsafe fn read_i64_at(ptr: *const u8, offset: usize) -> i64 {
    unsafe { std::ptr::read_unaligned(ptr.add(offset) as *const i64) }
}

fn serialize_value(ser: &mut Serializer, raw: i64, ty: &TypeShape) {
    match ty {
        TypeShape::Prim | TypeShape::Unit => {
            ser.write_i64(raw);
        }
        TypeShape::Text => {
            if raw == 0 {
                ser.write_nil();
            } else {
                let ptr = raw as *const std::os::raw::c_char;
                let str_bytes = unsafe { std::ffi::CStr::from_ptr(ptr).to_bytes() };
                let mut buf = Vec::with_capacity(str_bytes.len() + 1);
                buf.extend_from_slice(str_bytes);
                buf.push(0u8);
                let off = ser.write_appended(&buf);
                ser.write_appended_ptr(off);
            }
        }
        TypeShape::Bytes => {
            if raw == 0 {
                ser.write_i64(0); // len=0
            } else {
                let len = unsafe { read_i64_at(raw as *const u8, 0) };
                ser.write_i64(len);
                if len > 0 {
                    // Data do blob: offset 8, len bytes.
                    // Escreve na appended como raw bytes (sem nul terminator).
                    // Registra um appended_ptr para que from_bytes saiba onde está.
                    let data_ptr = unsafe { (raw as *const u8).add(8) };
                    let data = unsafe { std::slice::from_raw_parts(data_ptr, len as usize) };
                    let off = ser.write_appended(data);
                    ser.write_appended_ptr(off);
                }
            }
        }
        TypeShape::Tuple(elements) => {
            let ptr = raw as *const u8;
            for (i, elem_ty) in elements.iter().enumerate() {
                let word = unsafe { read_i64_at(ptr, i * 8) };
                serialize_value(ser, word, elem_ty);
            }
        }
        TypeShape::Struct(fields) => {
            let ptr = raw as *const u8;
            for (i, field_ty) in fields.iter().enumerate() {
                let word = unsafe { read_i64_at(ptr, i * 8) };
                serialize_value(ser, word, field_ty);
            }
        }
        TypeShape::Sum(variants) => {
            let ptr = raw as *const u8;
            let tag = unsafe { read_i64_at(ptr, 0) };
            let payload = unsafe { read_i64_at(ptr, 8) };
            ser.write_i64(tag);
            if let Some(Some(payload_ty)) = variants.get(tag as usize) {
                serialize_value(ser, payload, payload_ty);
            } else {
                ser.write_i64(payload);
            }
        }
        TypeShape::List(elem_ty) => {
            serialize_list(ser, raw, elem_ty);
        }
        TypeShape::Array(elem_ty) => {
            if raw == 0 {
                ser.write_i64(0);
                return;
            }
            let len = unsafe { read_i64_at(raw as *const u8, 0) };
            ser.write_i64(len);
            let ptr = raw as *const u8;
            for i in 0..len {
                let word = unsafe { read_i64_at(ptr, 8 + i as usize * 8) };
                serialize_value(ser, word, elem_ty);
            }
        }
    }
}

fn serialize_list(ser: &mut Serializer, ptr: i64, elem_ty: &TypeShape) {
    let mut current = ptr;
    while current != 0 {
        let raw_ptr = current as *const u8;
        let head = unsafe { read_i64_at(raw_ptr, 0) };
        let tail = unsafe { read_i64_at(raw_ptr, 8) };

        serialize_value(ser, head, elem_ty);

        if tail == 0 {
            ser.write_nil();
        } else {
            let next = ser.main_aligned_len() + 8;
            ser.write_main_ptr(next as i64);
        }
        current = tail;
    }
}

/// `kata_rt_to_bytes(value_ptr, type_id, arena_handle) -> bytes_ptr`
///
/// Serializa um valor em um blob `Bytes` com header estendido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_to_bytes(value_ptr: i64, type_id: i64, arena_handle: i64) -> i64 {
    let ty = match get_type_shape(type_id) {
        Some(t) => t,
        None => return 0,
    };

    let mut ser = Serializer::new();
    serialize_value(&mut ser, value_ptr, &ty);
    let (data, rebase_offsets) = ser.finish();

    let rebase_count = rebase_offsets.len() as i64;
    let header_size = 24 + rebase_offsets.len() * 8;
    let content_len = header_size + data.len();
    let blob_size = 8 + content_len; // Bytes header + conteúdo

    let ptr = crate::arena::kata_rt_arena_alloc(arena_handle, blob_size as i64);
    if ptr == 0 {
        return 0;
    }

    unsafe {
        let p = ptr as *mut u8;
        // Bytes header: content_len
        std::ptr::write_unaligned(p as *mut i64, content_len as i64);
        // Conteúdo:
        let c = p.add(8);
        std::ptr::write_unaligned(c as *mut i64, data.len() as i64); // data_len
        std::ptr::write_unaligned(c.add(8) as *mut i64, type_id); // type_id
        std::ptr::write_unaligned(c.add(16) as *mut i64, rebase_count); // rebase_count
        for (i, &off) in rebase_offsets.iter().enumerate() {
            std::ptr::write_unaligned(c.add(24 + i * 8) as *mut i64, off as i64);
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), c.add(header_size), data.len());
    }
    ptr
}

// ════════════════════════════════════════════════════════════
//  Desserialização (from_bytes)
// ════════════════════════════════════════════════════════════

/// Desserializador — lê bytes de um buffer e reconstrói na arena.
struct Deserializer<'a> {
    data: &'a [u8],
    rebase_offsets: &'a [usize],
    arena_handle: i64,
    pos: usize,
}

impl<'a> Deserializer<'a> {
    fn new(data: &'a [u8], rebase_offsets: &'a [usize], arena_handle: i64) -> Self {
        Deserializer {
            data,
            rebase_offsets,
            arena_handle,
            pos: 0,
        }
    }

    fn align(&mut self) {
        while !self.pos.is_multiple_of(8) && self.pos < self.data.len() {
            self.pos += 1;
        }
    }

    fn read_i64(&mut self) -> i64 {
        self.align();
        if self.pos + 8 > self.data.len() {
            return 0;
        }
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&self.data[self.pos..self.pos + 8]);
        self.pos += 8;
        i64::from_le_bytes(buf)
    }

    /// Lê um i64 que pode ser um ponteiro relativo. Faz rebasing se necessário.
    fn read_ptr(&mut self, base_ptr: i64) -> i64 {
        self.align();
        let offset_pos = self.pos;
        let raw = self.read_i64();
        if raw == 0 {
            return 0;
        }
        if self.rebase_offsets.contains(&offset_pos) {
            raw + base_ptr
        } else {
            raw
        }
    }

    /// Lê bytes da appended (offset absoluto dentro de data).
    fn slice_at(&self, offset: usize, len: usize) -> &'a [u8] {
        if offset + len <= self.data.len() {
            &self.data[offset..offset + len]
        } else {
            &[]
        }
    }

    /// Lê uma C string (nulo-terminada) da appended.
    fn read_cstr_at(&self, offset: usize) -> &'a [u8] {
        if offset >= self.data.len() {
            return &[];
        }
        let end = self.data[offset..]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.data.len() - offset);
        &self.data[offset..offset + end]
    }
}

fn deserialize_value(de: &mut Deserializer, ty: &TypeShape, base_ptr: i64) -> i64 {
    match ty {
        TypeShape::Prim => de.read_i64(),
        TypeShape::Unit => {
            let _ = de.read_i64();
            0
        }
        TypeShape::Text => {
            de.align();
            let offset_pos = de.pos;
            let raw = de.read_i64();
            if raw == 0 {
                return 0;
            }
            // raw é um offset absoluto dentro do buffer data (após finish()
            // somar main_len). É também marcado como rebase_offset.
            // Para encontrar a string: data[raw] = início da C string.
            // Mas raw foi ajustado por finish() para ser offset-absoluto.
            // No from_bytes, base_ptr é o ponteiro do data na memória.
            // O rebase_offset marca que raw precisa de rebasing (soma base_ptr)
            // para virar ponteiro absoluto na arena.
            // MAS: para Text, não queremos o ponteiro no buffer — queremos
            // copiar a string para a arena destino.
            // O offset da string dentro de data é raw (após finish()).
            // Se rebase_offset contém offset_pos, raw é offset relativo
            // que foi somado com main_len. Logo raw já é offset absoluto em data.
            let str_offset = if de.rebase_offsets.contains(&offset_pos) {
                raw as usize
            } else {
                raw as usize
            };
            let str_bytes = de.read_cstr_at(str_offset);
            let ptr =
                crate::arena::kata_rt_arena_alloc(de.arena_handle, (str_bytes.len() + 1) as i64);
            if ptr == 0 {
                return 0;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(str_bytes.as_ptr(), ptr as *mut u8, str_bytes.len());
                std::ptr::write((ptr as *mut u8).add(str_bytes.len()), 0);
            }
            ptr
        }
        TypeShape::Bytes => {
            de.align();
            let len_pos = de.pos;
            let _ = len_pos;
            let len = de.read_i64();
            if len <= 0 {
                return crate::bytes::kata_rt_bytes_alloc(0, de.arena_handle);
            }
            // O próximo i64 é um appended_ptr (offset para a data).
            de.align();
            let ptr_pos = de.pos;
            let data_offset = de.read_i64();
            // data_offset é um offset relativo (appended_ptr).
            // Somar base_ptr para obter offset absoluto no buffer.
            let abs = if de.rebase_offsets.contains(&ptr_pos) {
                (data_offset + base_ptr) as usize
            } else {
                data_offset as usize
            };
            let data = de.slice_at(abs, len as usize);
            // Alocar blob Bytes (8 + len) e copiar.
            let blob = crate::bytes::kata_rt_bytes_alloc(len, de.arena_handle);
            if blob == 0 {
                return 0;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    (blob as *mut u8).add(8),
                    len as usize,
                );
            }
            blob
        }
        TypeShape::Tuple(elements) => {
            let size = elements.len() * 8;
            let ptr = crate::arena::kata_rt_arena_alloc(de.arena_handle, size as i64);
            if ptr == 0 {
                return 0;
            }
            for (i, elem_ty) in elements.iter().enumerate() {
                let val = deserialize_value(de, elem_ty, base_ptr);
                unsafe {
                    std::ptr::write_unaligned((ptr as *mut u8).add(i * 8) as *mut i64, val);
                }
            }
            ptr
        }
        TypeShape::Struct(fields) => {
            let size = fields.len() * 8;
            let ptr = crate::arena::kata_rt_arena_alloc(de.arena_handle, size as i64);
            if ptr == 0 {
                return 0;
            }
            for (i, field_ty) in fields.iter().enumerate() {
                let val = deserialize_value(de, field_ty, base_ptr);
                unsafe {
                    std::ptr::write_unaligned((ptr as *mut u8).add(i * 8) as *mut i64, val);
                }
            }
            ptr
        }
        TypeShape::Sum(variants) => {
            let tag = de.read_i64();
            let payload_ty = variants.get(tag as usize).and_then(|v| v.as_ref());
            let payload = if let Some(pty) = payload_ty {
                deserialize_value(de, pty, base_ptr)
            } else {
                de.read_i64()
            };
            crate::sum::kata_rt_store_sum_result(tag, payload, de.arena_handle)
        }
        TypeShape::List(elem_ty) => deserialize_list(de, elem_ty, base_ptr),
        TypeShape::Array(elem_ty) => {
            let len = de.read_i64();
            if len <= 0 {
                return crate::array::kata_rt_array_alloc(0, de.arena_handle);
            }
            let arr = crate::array::kata_rt_array_alloc(len, de.arena_handle);
            if arr == 0 {
                return 0;
            }
            for i in 0..len {
                let val = deserialize_value(de, elem_ty, base_ptr);
                unsafe {
                    std::ptr::write_unaligned(
                        (arr as *mut u8).add(8 + i as usize * 8) as *mut i64,
                        val,
                    );
                }
            }
            arr
        }
    }
}

fn deserialize_list(de: &mut Deserializer, elem_ty: &TypeShape, base_ptr: i64) -> i64 {
    // Lista Cons: head (8 bytes) + tail (ptr|0).
    // tail é um main_ptr (offset relativo na main, rebasing +base_ptr).
    let mut cells: Vec<(i64, i64)> = Vec::new(); // (head_raw, head_value)
    loop {
        let head = deserialize_value(de, elem_ty, base_ptr);
        de.align();
        let tail_pos = de.pos;
        let tail_raw = de.read_i64();
        if tail_raw == 0 {
            // Nil — fim da lista.
            cells.push((head, 0));
            break;
        }
        // tail_raw é um offset relativo (main_ptr). Rebasing: +base_ptr.
        // Mas não precisamos seguir o ponteiro — os dados estão sequenciais
        // no buffer. A posição atual já aponta para a próxima cell.
        let _ = tail_pos;
        cells.push((head, -1)); // -1 = não-Nil, próxima cell segue
        // Se for um main_ptr com rebasing, o offset aponta para a próxima
        // cell que está logo após no buffer. Não precisamos pular —
        // a desserialização continua linearmente.
    }
    // Reconstrói a lista de trás pra frente.
    let mut tail = 0i64;
    for (head, marker) in cells.iter().rev() {
        let cell = crate::list::kata_rt_list_cons(*head, tail, de.arena_handle);
        if cell == 0 {
            return 0;
        }
        tail = cell;
        let _ = marker;
    }
    tail
}

/// `kata_rt_from_bytes(bytes_ptr, arena_handle) -> value_ptr`
///
/// Reconstrói um valor a partir de um blob `Bytes` produzido por `to_bytes`.
/// Lê `type_id` e `rebase_offsets` do header do blob.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_from_bytes(bytes_ptr: i64, arena_handle: i64) -> i64 {
    if bytes_ptr == 0 {
        return 0;
    }

    let ptr = bytes_ptr as *const u8;
    // Bytes header: content_len
    let content_len = unsafe { read_i64_at(ptr, 0) } as usize;
    if content_len < 24 {
        return 0;
    }

    let content = unsafe { ptr.add(8) };
    let data_len = unsafe { read_i64_at(content, 0) } as usize;
    let type_id = unsafe { read_i64_at(content, 8) };
    let rebase_count = unsafe { read_i64_at(content, 16) } as usize;

    let header_size = 24 + rebase_count * 8;
    if header_size > content_len {
        return 0;
    }

    // Lê rebase_offsets.
    let mut rebase_offsets: Vec<usize> = Vec::with_capacity(rebase_count);
    for i in 0..rebase_count {
        let off = unsafe { read_i64_at(content, 24 + i * 8) } as usize;
        rebase_offsets.push(off);
    }

    // data começa após o header do blob.
    let data_start = header_size;
    if data_start + data_len > content_len {
        return 0;
    }

    let ty = match get_type_shape(type_id) {
        Some(t) => t,
        None => return 0,
    };

    // O buffer de dados está em content + data_start.
    // base_ptr é o ponteiro base do buffer na arena atual (para rebasing
    // de ponteiros relativos). Como os dados estão no blob Bytes que
    // já está na arena, base_ptr = (content + data_start) as i64.
    let data_ptr = unsafe { content.add(data_start) };
    let data_slice = unsafe { std::slice::from_raw_parts(data_ptr, data_len) };
    let base_ptr = data_ptr as i64;

    let mut de = Deserializer::new(data_slice, &rebase_offsets, arena_handle);
    deserialize_value(&mut de, &ty, base_ptr)
}

// ════════════════════════════════════════════════════════════
//  Testes unitários
// ════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{kata_rt_arena_alloc, kata_rt_arena_create, kata_rt_arena_destroy};

    fn make_arena() -> i64 {
        kata_rt_arena_create()
    }

    fn register_types() {
        register_type_table(vec![
            TypeShape::Prim, // 0: Int
            TypeShape::Text, // 1: Text
            TypeShape::Tuple(vec![
                // 2: (Int, Text)
                TypeShape::Prim,
                TypeShape::Text,
            ]),
            TypeShape::List(Box::new(TypeShape::Prim)), // 3: List<Int>
        ]);
    }

    #[test]
    fn to_bytes_from_bytes_int() {
        register_types();
        let arena = make_arena();
        // Int 42 como SMI: (42 << 1) | 1 = 85
        let val = (42i64 << 1) | 1;
        let blob = kata_rt_to_bytes(val, 0, arena); // type_id=0 (Prim)
        assert!(blob != 0, "to_bytes produziu blob válido");
        let recovered = kata_rt_from_bytes(blob, arena);
        assert_eq!(recovered, val, "roundtrip Int SMI");
        kata_rt_arena_destroy(arena);
    }

    #[test]
    fn to_bytes_from_bytes_text() {
        register_types();
        let arena = make_arena();
        // Aloca "hello" como C string na arena.
        let s = b"hello\0";
        let text_ptr = kata_rt_arena_alloc(arena, s.len() as i64);
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), text_ptr as *mut u8, s.len());
        }
        let blob = kata_rt_to_bytes(text_ptr, 1, arena); // type_id=1 (Text)
        assert!(blob != 0, "to_bytes produziu blob válido");
        let recovered = kata_rt_from_bytes(blob, arena);
        assert!(recovered != 0, "from_bytes produziu ponteiro válido");
        // Verifica conteúdo.
        let cstr = unsafe { std::ffi::CStr::from_ptr(recovered as *const std::os::raw::c_char) };
        assert_eq!(cstr.to_bytes(), b"hello", "roundtrip Text");
        kata_rt_arena_destroy(arena);
    }

    #[test]
    fn to_bytes_from_bytes_tuple() {
        register_types();
        let arena = make_arena();
        // Tupla (42, "oi"):
        let tuple_ptr = kata_rt_arena_alloc(arena, 16);
        let s = b"oi\0";
        let text_ptr = kata_rt_arena_alloc(arena, s.len() as i64);
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), text_ptr as *mut u8, s.len());
            let smi = (42i64 << 1) | 1;
            std::ptr::write_unaligned(tuple_ptr as *mut i64, smi);
            std::ptr::write_unaligned((tuple_ptr as *mut u8).add(8) as *mut i64, text_ptr);
        }
        let blob = kata_rt_to_bytes(tuple_ptr, 2, arena); // type_id=2 (Tuple)
        assert!(blob != 0);
        let recovered = kata_rt_from_bytes(blob, arena);
        assert!(recovered != 0);
        // Lê campo 0 (Int SMI) e campo 1 (Text ptr).
        let field0 = unsafe { read_i64_at(recovered as *const u8, 0) };
        let field1 = unsafe { read_i64_at(recovered as *const u8, 8) };
        assert_eq!(field0, (42i64 << 1) | 1, "campo 0 = Int 42 SMI");
        assert!(field1 != 0, "campo 1 = Text ptr válido");
        let cstr = unsafe { std::ffi::CStr::from_ptr(field1 as *const std::os::raw::c_char) };
        assert_eq!(cstr.to_bytes(), b"oi", "campo 1 = \"oi\"");
        kata_rt_arena_destroy(arena);
    }

    #[test]
    fn to_bytes_from_bytes_list_int() {
        register_types();
        let arena = make_arena();
        // Lista [1, 2, 3] de Ints SMI.
        let mut list = 0i64;
        for &v in &[3i64, 2, 1] {
            let smi = (v << 1) | 1;
            list = crate::list::kata_rt_list_cons(smi, list, arena);
        }
        let blob = kata_rt_to_bytes(list, 3, arena); // type_id=3 (List<Int>)
        assert!(blob != 0);
        let recovered = kata_rt_from_bytes(blob, arena);
        assert!(recovered != 0);
        // Verifica: head=1, tail.head=2, tail.tail.head=3
        let h1 = crate::list::kata_rt_list_head(recovered);
        let t1 = crate::list::kata_rt_list_tail(recovered);
        let h2 = crate::list::kata_rt_list_head(t1);
        let t2 = crate::list::kata_rt_list_tail(t1);
        let h3 = crate::list::kata_rt_list_head(t2);
        assert_eq!(h1 >> 1, 1, "head = 1");
        assert_eq!(h2 >> 1, 2, "second = 2");
        assert_eq!(h3 >> 1, 3, "third = 3");
        kata_rt_arena_destroy(arena);
    }
}
