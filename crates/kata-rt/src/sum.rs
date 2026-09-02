//! Sum — alocação e introspecção de valores de enum com payload.
//!
//! `kata_rt_store_sum_result(tag, payload)` aloca 16 bytes na arena global
//! (8 para tag + 8 para payload) e retorna o ponteiro.
//! `kata_rt_sum_tag_int(val)` extrai a tag de um Sum box.
//!
//! Layout do box:
//! ```text
//! offset 0: tag (i64) — índice da variante no enum
//! offset 8: payload (i64) — valor do payload (SMI, ptr, etc.)
//! ```

use std::ffi::CString;

/// Aloca 16 bytes na arena especificada, armazena tag e payload, retorna ponteiro.
///
/// Pré-11: `arena_handle` substitui o handle 0 hardcoded.
///
/// # Safety
/// `tag` e `payload` são valores i64 válidos.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_store_sum_result(tag: i64, payload: i64, arena_handle: i64) -> i64 {
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 16);
    if ptr == 0 {
        return 0; // falha na alocação
    }
    // Store tag no offset 0, payload no offset 8.
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, tag);
        std::ptr::write_unaligned((ptr as *mut u8).add(8) as *mut i64, payload);
    }
    ptr
}

/// Extrai a tag de um Sum box (lê os primeiros 8 bytes).
///
/// # Safety
/// `val` deve ser um ponteiro válido retornado por `kata_rt_store_sum_result`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_sum_tag_int(val: i64) -> i64 {
    if val == 0 {
        return 0;
    }
    unsafe { std::ptr::read_unaligned(val as *const i64) }
}

/// Constrói um `Err` com mensagem de erro Text. Aloca uma C string no
/// heap (via `into_raw` — mesmo padrão de `Ok` payloads), armazena como
/// payload do Sum box (tag=1). Garante que `Err` nunca tenha payload
/// null — o contrato de `Err` exige um Text.
///
/// Usado por todas as operações de runtime que retornam `Result::Err`
/// (at, list_get, array_get, dict_get, etc.) em vez de
/// `store_sum_result(1, 0, arena)` que produzia payload null → SIGSEGV
/// no `show`.
pub(crate) fn err_with_msg(msg: &str, arena_handle: i64) -> i64 {
    let cstr = CString::new(msg)
        .unwrap_or_else(|_| CString::new("").expect("string vazia é CString válida"));
    let ptr = cstr.into_raw();
    kata_rt_store_sum_result(1, ptr as i64, arena_handle)
}
