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
        return crate::sum::err_with_msg("at: texto nulo", arena_handle);
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
        return crate::sum::err_with_msg("at: índice fora dos limites", arena_handle);
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
#[path = "slice_tests.rs"]
mod tests;
