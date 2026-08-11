//! Desserialização (from_bytes) — ver `super` para a documentação do layout.
//!
//! A2: `kata_rt_from_bytes` agora recebe `rt` como primeiro parâmetro. As
//! funções internas recebem `rt` para chamar as FFIs de alocação.

use super::{TypeShape, get_type_shape, read_i64_at};

/// Desserializador — lê bytes de um buffer e reconstrói na arena.
struct Deserializer<'a> {
    data: &'a [u8],
    rebase_offsets: &'a [usize],
    rt: i64,
    arena_handle: i64,
    pos: usize,
}

impl<'a> Deserializer<'a> {
    fn new(data: &'a [u8], rebase_offsets: &'a [usize], rt: i64, arena_handle: i64) -> Self {
        Deserializer {
            data,
            rebase_offsets,
            rt,
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

    #[allow(dead_code)]
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

    fn slice_at(&self, offset: usize, len: usize) -> &'a [u8] {
        if offset + len <= self.data.len() {
            &self.data[offset..offset + len]
        } else {
            &[]
        }
    }

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
            let str_offset = raw as usize;
            let str_bytes = de.read_cstr_at(str_offset);
            let ptr = crate::arena::kata_rt_arena_alloc(
                de.rt,
                de.arena_handle,
                (str_bytes.len() + 1) as i64,
            );
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
            let _len_pos = de.pos;
            let len = de.read_i64();
            if len <= 0 {
                return crate::bytes::kata_rt_bytes_alloc(0, de.arena_handle);
            }
            de.align();
            let ptr_pos = de.pos;
            let data_offset = de.read_i64();
            let abs = if de.rebase_offsets.contains(&ptr_pos) {
                (data_offset + base_ptr) as usize
            } else {
                data_offset as usize
            };
            let data = de.slice_at(abs, len as usize);
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
            let ptr = crate::arena::kata_rt_arena_alloc(de.rt, de.arena_handle, size as i64);
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
            let ptr = crate::arena::kata_rt_arena_alloc(de.rt, de.arena_handle, size as i64);
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
    let mut cells: Vec<(i64, i64)> = Vec::new();
    loop {
        let head = deserialize_value(de, elem_ty, base_ptr);
        de.align();
        let _tail_pos = de.pos;
        let tail_raw = de.read_i64();
        if tail_raw == 0 {
            cells.push((head, 0));
            break;
        }
        cells.push((head, -1));
    }
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

/// `kata_rt_from_bytes(rt, bytes_ptr, arena_handle) -> value_ptr`
///
/// Reconstrói um valor a partir de um blob `Bytes` produzido por `to_bytes`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_from_bytes(rt: i64, bytes_ptr: i64, arena_handle: i64) -> i64 {
    if bytes_ptr == 0 {
        return 0;
    }

    let ptr = bytes_ptr as *const u8;
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

    let mut rebase_offsets: Vec<usize> = Vec::with_capacity(rebase_count);
    for i in 0..rebase_count {
        let off = unsafe { read_i64_at(content, 24 + i * 8) } as usize;
        rebase_offsets.push(off);
    }

    let data_start = header_size;
    if data_start + data_len > content_len {
        return 0;
    }

    let ty = match get_type_shape(rt, type_id) {
        Some(t) => t,
        None => return 0,
    };

    let data_ptr = unsafe { content.add(data_start) };
    let data_slice = unsafe { std::slice::from_raw_parts(data_ptr, data_len) };
    let base_ptr = data_ptr as i64;

    let mut de = Deserializer::new(data_slice, &rebase_offsets, rt, arena_handle);
    deserialize_value(&mut de, &ty, base_ptr)
}
