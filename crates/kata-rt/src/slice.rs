//! Slice / index de coleções (Text, Array, List).
//!
//! Extraído de `bytes.rs` — estas operações não são de Bytes, são operações de
//! slice/index de coleções (Text por codepoints, Array, List) que viviam em
//! `bytes.rs` por conveniência temporária durante o PRD-bytes.

use std::ffi::CString;

use crate::bytes::{tag_smi, untag_smi};

// ── Text como INDEXABLE/COUNTABLE/SLICEABLE (codepoints) ───

/// Codepoint em índice N de uma Text (C string). Retorna Result box (Sum):
/// - Ok: tag=0, payload=ponteiro para C string contendo 1 codepoint
/// - Err: tag=1, payload=0 (out of bounds)
///
/// # Safety
/// `text_ptr` deve ser uma C string válida (nulo-terminada) ou 0.
/// `arena_handle` deve ser válido (para alocar a C string do codepoint).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_text_at(text_ptr: i64, idx: i64, arena_handle: i64) -> i64 {
    if text_ptr == 0 {
        return crate::sum::kata_rt_store_sum_result(1, 0, arena_handle);
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(text_ptr as *const std::os::raw::c_char) };
    let s = cstr.to_str().unwrap_or("");
    // Coleta codepoints em uma Vec para indexação O(1).
    let codepoints: Vec<char> = s.chars().collect();
    let len = codepoints.len() as i64;
    // idx é SMI-tagged (vindo do codegen). Untag antes de usar.
    let idx = untag_smi(idx);
    // Suporte a índice negativo.
    let real_idx = if idx < 0 { len + idx } else { idx };
    if real_idx < 0 || real_idx >= len {
        return crate::sum::kata_rt_store_sum_result(1, 0, arena_handle);
    }
    let c = codepoints[real_idx as usize];
    let cstr_ptr = CString::new(c.to_string())
        .unwrap_or_else(|_| CString::new("").expect("CString vazia sempre válida"))
        .into_raw();
    crate::sum::kata_rt_store_sum_result(0, cstr_ptr as i64, arena_handle)
}

/// Número de codepoints Unicode de uma Text (C string). SMI-tagged.
///
/// # Safety
/// `text_ptr` deve ser uma C string válida (nulo-terminada) ou 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_text_len(text_ptr: i64) -> i64 {
    if text_ptr == 0 {
        return tag_smi(0);
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(text_ptr as *const std::os::raw::c_char) };
    let s = cstr.to_str().unwrap_or("");
    let count = s.chars().count() as i64;
    tag_smi(count)
}

/// Slice de Text por codepoints: start (inclusive) até end (exclusive).
/// Retorna C string (ponteiro, ownership transferida).
///
/// # Safety
/// `text_ptr` deve ser uma C string válida (nulo-terminada) ou 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_text_slice(
    text_ptr: i64,
    start: i64,
    end: i64,
) -> *mut std::os::raw::c_char {
    if text_ptr == 0 {
        return CString::new("")
            .expect("CString vazia sempre válida")
            .into_raw();
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(text_ptr as *const std::os::raw::c_char) };
    let s = cstr.to_str().unwrap_or("");
    // Coleta codepoints para slice por codepoint (não por byte).
    let codepoints: Vec<char> = s.chars().collect();
    let len = codepoints.len() as i64;
    // start e end são SMI-tagged (vindo do codegen). Untag antes de usar.
    let start = untag_smi(start);
    let end = untag_smi(end);
    // Normaliza índices negativos.
    let start = if start < 0 { len + start } else { start };
    let end = if end < 0 { len + end } else { end };
    // Clamp.
    let start = start.clamp(0, len);
    let end = end.clamp(0, len);
    if start >= end {
        return CString::new("")
            .expect("CString vazia sempre válida")
            .into_raw();
    }
    let sub: String = codepoints[start as usize..end as usize].iter().collect();
    CString::new(sub)
        .unwrap_or_else(|_| CString::new("").expect("CString vazia sempre válida"))
        .into_raw()
}

// ── Slice de Array e List ──────────────────────────────────

/// Slice de Array: start (inclusive) até end (exclusive). Retorna novo Array.
///
/// Layout de Array: [len: i64][data: i64 * len]
///
/// # Safety
/// `ptr` deve ser um Array válido. `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_array_slice(
    ptr: i64,
    start: i64,
    end: i64,
    arena_handle: i64,
) -> i64 {
    // start e end são SMI-tagged (vindo do codegen). Untag antes de usar.
    let start = untag_smi(start);
    let end = untag_smi(end);
    if ptr == 0 {
        return crate::array::kata_rt_array_alloc(0, arena_handle);
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    let start = if start < 0 { len + start } else { start };
    let end = if end < 0 { len + end } else { end };
    let start = start.clamp(0, len);
    let end = end.clamp(0, len);
    if start >= end {
        return crate::array::kata_rt_array_alloc(0, arena_handle);
    }
    let sub_len = end - start;
    let new_ptr = crate::array::kata_rt_array_alloc(sub_len, arena_handle);
    if new_ptr == 0 {
        return 0;
    }
    for i in 0..sub_len {
        let val = unsafe {
            std::ptr::read_unaligned(
                (ptr as *const u8).add((8 + (start + i) * 8) as usize) as *const i64
            )
        };
        crate::array::kata_rt_array_set(new_ptr, i, val);
    }
    new_ptr
}

/// Slice de List: start (inclusive) até end (exclusive). Retorna nova List.
///
/// # Safety
/// `ptr` deve ser uma List válida (Cons cell ou 0=Nil). `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_list_slice(
    ptr: i64,
    start: i64,
    end: i64,
    arena_handle: i64,
) -> i64 {
    // start e end são SMI-tagged (vindo do codegen). Untag antes de usar.
    let start = untag_smi(start);
    let end = untag_smi(end);
    if ptr == 0 {
        return 0; // Nil
    }
    // Coleta elementos da lista.
    let mut elements = Vec::new();
    let mut current = ptr;
    while current != 0 {
        let head = unsafe { std::ptr::read_unaligned(current as *const i64) };
        let tail = unsafe { std::ptr::read_unaligned((current as *const u8).add(8) as *const i64) };
        elements.push(head);
        current = tail;
    }
    let len = elements.len() as i64;
    let start = if start < 0 { len + start } else { start };
    let end = if end < 0 { len + end } else { end };
    let start = start.clamp(0, len);
    let end = end.clamp(0, len);
    if start >= end {
        return 0; // Nil
    }
    // Constrói nova lista (cons de trás pra frente).
    let mut result = 0i64;
    for i in (start..end).rev() {
        result = crate::list::kata_rt_list_cons(elements[i as usize], result, arena_handle);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::kata_rt_arena_create;
    use crate::bytes::{tag_smi, untag_smi};

    struct TestRt {
        rt_ptr: i64,
    }
    impl TestRt {
        fn new() -> Self {
            let rt = Box::new(crate::runtime::Runtime::new());
            let ptr = Box::into_raw(rt) as i64;
            crate::arena::set_rt_ptr(ptr);
            TestRt { rt_ptr: ptr }
        }
    }
    impl Drop for TestRt {
        fn drop(&mut self) {
            unsafe {
                drop(Box::from_raw(self.rt_ptr as *mut crate::runtime::Runtime));
            }
        }
    }

    fn make_arena() -> (TestRt, i64) {
        let rt = TestRt::new();
        let arena = kata_rt_arena_create(rt.rt_ptr);
        (rt, arena)
    }

    #[test]
    fn text_len_codepoints() {
        let text = CString::new("Olá").unwrap(); // O, l, á = 3 codepoints, 4 bytes
        let len = unsafe { kata_rt_text_len(text.as_ptr() as i64) };
        assert_eq!(untag_smi(len), 3); // codepoints, não bytes
    }

    #[test]
    fn text_len_emoji() {
        let text = CString::new("a🚀b").unwrap(); // a, 🚀, b = 3 codepoints, 6 bytes
        let len = unsafe { kata_rt_text_len(text.as_ptr() as i64) };
        assert_eq!(untag_smi(len), 3);
    }

    #[test]
    fn text_at_basic() {
        let (_rt, arena) = make_arena();
        let text = CString::new("ABC").unwrap();
        let result = unsafe { kata_rt_text_at(text.as_ptr() as i64, tag_smi(0), arena) };
        let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
        let payload =
            unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
        assert_eq!(tag, 0); // Ok
        let s = unsafe {
            std::ffi::CStr::from_ptr(payload as *const std::os::raw::c_char)
                .to_str()
                .unwrap()
        };
        assert_eq!(s, "A");
    }

    #[test]
    fn text_at_unicode() {
        let (_rt, arena) = make_arena();
        let text = CString::new("Olá").unwrap();
        let result = unsafe { kata_rt_text_at(text.as_ptr() as i64, tag_smi(2), arena) };
        let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
        let payload =
            unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
        assert_eq!(tag, 0); // Ok
        let s = unsafe {
            std::ffi::CStr::from_ptr(payload as *const std::os::raw::c_char)
                .to_str()
                .unwrap()
        };
        assert_eq!(s, "á");
    }

    #[test]
    fn text_at_out_of_bounds() {
        let (_rt, arena) = make_arena();
        let text = CString::new("AB").unwrap();
        let result = unsafe { kata_rt_text_at(text.as_ptr() as i64, tag_smi(5), arena) };
        let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
        assert_eq!(tag, 1); // Err
    }

    #[test]
    fn text_slice_codepoints() {
        let text = CString::new("Hello").unwrap();
        let result = unsafe { kata_rt_text_slice(text.as_ptr() as i64, tag_smi(1), tag_smi(4)) };
        let s = unsafe { std::ffi::CStr::from_ptr(result).to_str().unwrap() };
        assert_eq!(s, "ell");
        unsafe { _ = CString::from_raw(result) };
    }

    #[test]
    fn text_slice_unicode() {
        let text = CString::new("Olá mundo").unwrap();
        let result = unsafe { kata_rt_text_slice(text.as_ptr() as i64, tag_smi(0), tag_smi(3)) };
        let s = unsafe { std::ffi::CStr::from_ptr(result).to_str().unwrap() };
        assert_eq!(s, "Olá");
        unsafe { _ = CString::from_raw(result) };
    }

    #[test]
    fn array_slice() {
        let (_rt, arena) = make_arena();
        let arr = crate::array::kata_rt_array_alloc(5, arena);
        crate::array::kata_rt_array_set(arr, 0, tag_smi(10));
        crate::array::kata_rt_array_set(arr, 1, tag_smi(20));
        crate::array::kata_rt_array_set(arr, 2, tag_smi(30));
        crate::array::kata_rt_array_set(arr, 3, tag_smi(40));
        crate::array::kata_rt_array_set(arr, 4, tag_smi(50));
        let sub = unsafe { kata_rt_array_slice(arr, tag_smi(1), tag_smi(3), arena) };
        assert_eq!(untag_smi(crate::array::kata_rt_array_len(sub)), 2);
        assert_eq!(untag_smi(crate::array::kata_rt_array_get(sub, 0)), 20);
        assert_eq!(untag_smi(crate::array::kata_rt_array_get(sub, 1)), 30);
    }

    #[test]
    fn list_slice() {
        let (_rt, arena) = make_arena();
        // Constrói lista [10, 20, 30, 40, 50]
        let mut list = 0i64;
        for &v in &[50, 40, 30, 20, 10] {
            list = crate::list::kata_rt_list_cons(tag_smi(v), list, arena);
        }
        let sub = unsafe { kata_rt_list_slice(list, tag_smi(1), tag_smi(3), arena) };
        // Deveria ser [20, 30]
        let h1 = unsafe { std::ptr::read_unaligned(sub as *const i64) };
        let t1 = unsafe { std::ptr::read_unaligned((sub as *const u8).add(8) as *const i64) };
        assert_eq!(untag_smi(h1), 20);
        let h2 = unsafe { std::ptr::read_unaligned(t1 as *const i64) };
        assert_eq!(untag_smi(h2), 30);
    }
}
