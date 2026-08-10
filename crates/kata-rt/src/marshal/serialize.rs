//! Serialização (to_bytes) — ver `super` para a documentação do layout.
//!
//! A2: `kata_rt_to_bytes` agora recebe `rt` como primeiro parâmetro. As
//! funções internas `serialize_value` e `serialize_list` recebem `rt` para
//! poder chamar `get_type_shape(rt, ...)` quando encontram tipos aninhados.

use super::{TypeShape, get_type_shape, read_i64_at};

/// Serializador — duas regiões (main + appended), igual ao comptime.
struct Serializer {
    main: Vec<u8>,
    appended: Vec<u8>,
    rebase_offsets: Vec<usize>,
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
        self.main.extend_from_slice(&(appended_offset as i64).to_le_bytes());
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

fn serialize_value(ser: &mut Serializer, rt: i64, raw: i64, ty: &TypeShape) {
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
                ser.write_i64(0);
            } else {
                let len = unsafe { read_i64_at(raw as *const u8, 0) };
                ser.write_i64(len);
                if len > 0 {
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
                serialize_value(ser, rt, word, elem_ty);
            }
        }
        TypeShape::Struct(fields) => {
            let ptr = raw as *const u8;
            for (i, field_ty) in fields.iter().enumerate() {
                let word = unsafe { read_i64_at(ptr, i * 8) };
                serialize_value(ser, rt, word, field_ty);
            }
        }
        TypeShape::Sum(variants) => {
            let ptr = raw as *const u8;
            let tag = unsafe { read_i64_at(ptr, 0) };
            let payload = unsafe { read_i64_at(ptr, 8) };
            ser.write_i64(tag);
            if let Some(Some(payload_ty)) = variants.get(tag as usize) {
                serialize_value(ser, rt, payload, payload_ty);
            } else {
                ser.write_i64(payload);
            }
        }
        TypeShape::List(elem_ty) => {
            serialize_list(ser, rt, raw, elem_ty);
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
                serialize_value(ser, rt, word, elem_ty);
            }
        }
    }
}

fn serialize_list(ser: &mut Serializer, rt: i64, ptr: i64, elem_ty: &TypeShape) {
    let mut current = ptr;
    while current != 0 {
        let raw_ptr = current as *const u8;
        let head = unsafe { read_i64_at(raw_ptr, 0) };
        let tail = unsafe { read_i64_at(raw_ptr, 8) };

        serialize_value(ser, rt, head, elem_ty);

        if tail == 0 {
            ser.write_nil();
        } else {
            let next = ser.main_aligned_len() + 8;
            ser.write_main_ptr(next as i64);
        }
        current = tail;
    }
}

/// `kata_rt_to_bytes(rt, value_ptr, type_id, arena_handle) -> bytes_ptr`
///
/// Serializa um valor em um blob `Bytes` com header estendido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_to_bytes(rt: i64, value_ptr: i64, type_id: i64, arena_handle: i64) -> i64 {
    let ty = match get_type_shape(rt, type_id) {
        Some(t) => t,
        None => return 0,
    };

    let mut ser = Serializer::new();
    serialize_value(&mut ser, rt, value_ptr, &ty);
    let (data, rebase_offsets) = ser.finish();

    let rebase_count = rebase_offsets.len() as i64;
    let header_size = 24 + rebase_offsets.len() * 8;
    let content_len = header_size + data.len();
    let blob_size = 8 + content_len;

    let ptr = crate::arena::kata_rt_arena_alloc(rt, arena_handle, blob_size as i64);
    if ptr == 0 {
        return 0;
    }

    unsafe {
        let p = ptr as *mut u8;
        std::ptr::write_unaligned(p as *mut i64, content_len as i64);
        let c = p.add(8);
        std::ptr::write_unaligned(c as *mut i64, data.len() as i64);
        std::ptr::write_unaligned(c.add(8) as *mut i64, type_id);
        std::ptr::write_unaligned(c.add(16) as *mut i64, rebase_count);
        for (i, &off) in rebase_offsets.iter().enumerate() {
            std::ptr::write_unaligned(c.add(24 + i * 8) as *mut i64, off as i64);
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), c.add(header_size), data.len());
    }
    ptr
}