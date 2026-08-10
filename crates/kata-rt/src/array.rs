//! Array — vetor contíguo alocado na arena.
//!
//! Layout (8 + len*8 bytes):
//! ```text
//! offset 0:        len (i64) — número de elementos
//! offset 8+i*8:    element[i] (i64) — i-ésimo elemento
//! ```

use crate::bytes::untag_smi;

/// Aloca um array com `len` elementos na arena especificada.
/// O array é zerado (todos elementos = 0). Use `kata_rt_array_set` para preencher.
///
/// # Safety
/// `len` deve ser >= 0. `arena_handle` é um handle válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_array_alloc(len: i64, arena_handle: i64) -> i64 {
    if len < 0 {
        return 0;
    }
    let size = 8 + len * 8;
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, size);
    if ptr == 0 {
        return 0;
    }
    // Store len no offset 0.
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, len);
    }
    ptr
}

/// Retorna o número de elementos do array (load offset 0). SMI-tagged.
///
/// # Safety
/// `ptr` deve ser um ponteiro válido retornado por `kata_rt_array_alloc`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_array_len(ptr: i64) -> i64 {
    if ptr == 0 {
        return 1; // SMI(0) = (0 << 1) | 1 = 1
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    (len << 1) | 1
}

/// Retorna o elemento no índice `idx` (load ptr+8+idx*8). Sem bounds check.
///
/// # Safety
/// `ptr` deve ser válido e `idx` deve estar em [0, len).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_array_get(ptr: i64, idx: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    let offset = 8 + idx * 8;
    unsafe { std::ptr::read_unaligned((ptr as *const u8).add(offset as usize) as *const i64) }
}

/// Armazena `val` no índice `idx` (store ptr+8+idx*8). Sem bounds check.
///
/// # Safety
/// `ptr` deve ser válido e `idx` deve estar em [0, len).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_array_set(ptr: i64, idx: i64, val: i64) {
    if ptr == 0 {
        return;
    }
    let offset = 8 + idx * 8;
    unsafe { std::ptr::write_unaligned((ptr as *mut u8).add(offset as usize) as *mut i64, val) }
}

/// Acesso por índice com bounds check. Retorna um Result box (Sum):
/// - Ok: tag=0, payload=valor
/// - Err: tag=1, payload=0 (out of bounds)
///
/// Layout do Result box: igual a store_sum_result (16 bytes).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_array_get_checked(ptr: i64, idx: i64) -> i64 {
    // idx é SMI-tagged (vindo do codegen). Untag antes de usar.
    let idx = untag_smi(idx);
    if ptr == 0 {
        return crate::sum::kata_rt_store_sum_result(1, 0, 0);
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    if idx < 0 || idx >= len {
        return crate::sum::kata_rt_store_sum_result(1, 0, 0);
    }
    let offset = 8 + idx * 8;
    let val =
        unsafe { std::ptr::read_unaligned((ptr as *const u8).add(offset as usize) as *const i64) };
    crate::sum::kata_rt_store_sum_result(0, val, 0)
}

/// Verifica se `item` está no array (percorre elementos). Retorna 1 (true) ou 0 (false).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_array_contains(ptr: i64, item: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    for i in 0..len {
        let offset = 8 + i * 8;
        let val = unsafe {
            std::ptr::read_unaligned((ptr as *const u8).add(offset as usize) as *const i64)
        };
        if val == item {
            return 1;
        }
    }
    0
}
