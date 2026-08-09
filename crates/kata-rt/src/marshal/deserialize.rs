//! Desserialização (from_bytes) — ver `super` para a documentação do layout.

use super::{TypeShape, get_type_shape, read_i64_at};
use crate::arena::kata_rt_arena_alloc;
use crate::array::kata_rt_array_alloc;
use crate::bytes::kata_rt_bytes_alloc;
use crate::list::kata_rt_list_cons;
use crate::sum::kata_rt_store_sum_result;

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
    #[allow(dead_code)] // reservado para futura deserialização IPC
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
            let _offset_pos = de.pos;
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
            let str_offset = raw as usize;
            let str_bytes = de.read_cstr_at(str_offset);
            let ptr = kata_rt_arena_alloc(de.arena_handle, (str_bytes.len() + 1) as i64);
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
                return kata_rt_bytes_alloc(0, de.arena_handle);
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
            let blob = kata_rt_bytes_alloc(len, de.arena_handle);
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
            let ptr = kata_rt_arena_alloc(de.arena_handle, size as i64);
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
            let ptr = kata_rt_arena_alloc(de.arena_handle, size as i64);
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
            kata_rt_store_sum_result(tag, payload, de.arena_handle)
        }
        TypeShape::List(elem_ty) => deserialize_list(de, elem_ty, base_ptr),
        TypeShape::Array(elem_ty) => {
            let len = de.read_i64();
            if len <= 0 {
                return kata_rt_array_alloc(0, de.arena_handle);
            }
            let arr = kata_rt_array_alloc(len, de.arena_handle);
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
        let cell = kata_rt_list_cons(*head, tail, de.arena_handle);
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
