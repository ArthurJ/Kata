//! Conversões entre Bytes e outros tipos primitivos.
//!
//! Pontes: Int → Bytes (little-endian), Text ↔ Bytes (UTF-8),
//! Int → Byte (escalar). Estas funções alocam/manipulam blobs via
//! `kata_rt_bytes_alloc` e copiam dados entre representações.

use std::ffi::CString;

use crate::bytes::{kata_rt_bytes_alloc, untag_smi};

/// Int → Bytes (4 bytes little-endian). Retorna ponteiro para blob.
///
/// # Safety
/// `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_int_to_bytes(n: i64, arena_handle: i64) -> i64 {
    let n = untag_smi(n);
    let bytes = n.to_le_bytes();
    let ptr = kata_rt_bytes_alloc(4, arena_handle);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (ptr as *mut u8).add(8), 4);
    }
    ptr
}

/// Text (C string) → Bytes (codifica UTF-8 — o conteúdo já é UTF-8).
/// Retorna ponteiro para blob.
///
/// # Safety
/// `text_ptr` deve ser um ponteiro C string válido (nulo-terminado) ou 0.
/// `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_text_to_bytes(text_ptr: i64, arena_handle: i64) -> i64 {
    if text_ptr == 0 {
        return kata_rt_bytes_alloc(0, arena_handle);
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(text_ptr as *const std::os::raw::c_char) };
    let bytes = cstr.to_bytes();
    let len = bytes.len() as i64;
    let blob = kata_rt_bytes_alloc(len, arena_handle);
    if blob == 0 {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (blob as *mut u8).add(8), len as usize);
    }
    blob
}

/// Bytes → Text (decodifica UTF-8). Retorna C string (ponteiro).
/// Se os bytes não forem UTF-8 válido, retorna string vazia.
///
/// # Safety
/// `bytes_ptr` deve ser um ponteiro válido retornado por `kata_rt_bytes_*`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_to_text(bytes_ptr: i64) -> *mut std::os::raw::c_char {
    if bytes_ptr == 0 {
        return CString::new("")
            .expect("CString vazia sempre válida")
            .into_raw();
    }
    let len = unsafe { std::ptr::read_unaligned(bytes_ptr as *const i64) };
    if len <= 0 {
        return CString::new("")
            .expect("CString vazia sempre válida")
            .into_raw();
    }
    let bytes =
        unsafe { std::slice::from_raw_parts((bytes_ptr as *const u8).add(8), len as usize) };
    let text = std::str::from_utf8(bytes).unwrap_or("");
    CString::new(text)
        .unwrap_or_else(|_| CString::new("").expect("CString vazia sempre válida"))
        .into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::kata_rt_arena_create;
    use crate::bytes::{kata_rt_bytes_get, kata_rt_bytes_len, tag_smi};

    fn make_arena() -> i64 {
        kata_rt_arena_create()
    }

    #[test]
    fn int_to_bytes() {
        let arena = make_arena();
        let ptr = kata_rt_int_to_bytes(tag_smi(42), arena);
        assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 4);
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 0)), 0x2A); // 42 = 0x2A
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 1)), 0x00);
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 2)), 0x00);
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 3)), 0x00);
    }

    #[test]
    fn text_to_bytes_and_back() {
        let arena = make_arena();
        let text = CString::new("Hello").unwrap();
        let text_ptr = text.as_ptr() as i64;
        let bytes_ptr = unsafe { kata_rt_text_to_bytes(text_ptr, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_len(bytes_ptr)), 5);
        assert_eq!(untag_smi(kata_rt_bytes_get(bytes_ptr, 0)), 0x48);
        // Convert back to text.
        let result_ptr = unsafe { kata_rt_bytes_to_text(bytes_ptr) };
        let s = unsafe { std::ffi::CStr::from_ptr(result_ptr).to_str().unwrap() };
        assert_eq!(s, "Hello");
        unsafe { _ = CString::from_raw(result_ptr) };
    }
}
