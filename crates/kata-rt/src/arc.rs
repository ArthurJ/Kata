//! Arc<T> — CaptureBox para closures com captura.
//!
//! `kata_rt_alloc_arc(fn_ptr, captures_ptr, n_captures, arena_handle)` aloca
//! um CaptureBox na arena especificada. O box contém:
//!
//! ```text
//! offset 0:  fn_ptr (i64) — ponteiro da função JIT
//! offset 8:  refcount (i64) — contagem de referências (sempre ≥1)
//! offset 16: n_captures (i64) — número de captures (para dealloc)
//! offset 24: captures[0] (i64)
//! offset 32: captures[1] (i64)
//! ...
//! offset 24 + (n-1)*8: captures[n-1]
//! ```
//!
//! `kata_rt_incref(box_ptr)` incrementa o refcount.
//! `kata_rt_decref(box_ptr)` decrementa o refcount. Quando chega a 0,
//! o box é liberado individualmente da root arena via
//! `kata_rt_arena_dealloc(root_arena_handle, box_ptr, size)`.
//!
//! Layout: 24 bytes de header + n_captures * 8 bytes de captures data.
//! O `n_captures` no header permite que `decref` calcule o size para
//! `std::alloc::dealloc` (que requer o `Layout` exato).

use crate::arena::kata_rt_get_root_arena_handle;

/// Offset do `n_captures` no header do CaptureBox.
const N_CAPTURES_OFFSET: usize = 16;
/// Offset do primeiro capture no CaptureBox.
const CAPTURES_OFFSET: usize = 24;
/// Tamanho do header (fn_ptr + refcount + n_captures).
const HEADER_SIZE: usize = 24;

/// Aloca um CaptureBox na arena especificada e retorna o ponteiro.
///
/// `fn_ptr` é o ponteiro da função JIT (para `call_indirect`).
/// `captures_ptr` é um ponteiro para um array de i64 com os valores capturados.
/// `n_captures` é o número de valores capturados.
/// `arena_handle` é o handle da arena onde o box é alocado.
///
/// # Safety
/// `captures_ptr` deve ser um ponteiro válido para `n_captures` i64s,
/// ou 0/null se `n_captures == 0`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_alloc_arc(
    fn_ptr: i64,
    captures_ptr: i64,
    n_captures: i64,
    arena_handle: i64,
) -> i64 {
    if n_captures < 0 {
        return 0;
    }

    // Tamanho: 24 bytes header + n_captures * 8 bytes
    let total_size = HEADER_SIZE as i64 + n_captures * 8;

    let box_ptr = crate::arena::kata_rt_arena_alloc(arena_handle, total_size);
    if box_ptr == 0 {
        return 0; // falha na alocação
    }

    unsafe {
        let ptr = box_ptr as *mut u8;
        // Header: fn_ptr no offset 0
        std::ptr::write_unaligned(ptr as *mut i64, fn_ptr);
        // Header: refcount = 1 no offset 8
        std::ptr::write_unaligned(ptr.add(8) as *mut i64, 1);
        // Header: n_captures no offset 16 (Fio 16 — para dealloc)
        std::ptr::write_unaligned(ptr.add(N_CAPTURES_OFFSET) as *mut i64, n_captures);

        // Captures: copia do array de origem para o box (offset 24)
        if n_captures > 0 && captures_ptr != 0 {
            let src = captures_ptr as *const i64;
            let dst = ptr.add(CAPTURES_OFFSET) as *mut i64;
            for i in 0..n_captures as usize {
                let val = std::ptr::read_unaligned(src.add(i));
                std::ptr::write_unaligned(dst.add(i), val);
            }
        }
    }

    box_ptr
}

/// Incrementa o refcount de um CaptureBox.
///
/// # Safety
/// `box_ptr` deve ser um ponteiro válido retornado por `kata_rt_alloc_arc`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_incref(box_ptr: i64) -> i64 {
    if box_ptr == 0 {
        return 0;
    }
    unsafe {
        let refcount_ptr = (box_ptr as *mut u8).add(8) as *mut i64;
        let count = std::ptr::read_unaligned(refcount_ptr);
        std::ptr::write_unaligned(refcount_ptr, count + 1);
    }
    0
}

/// Decrementa o refcount de um CaptureBox.
///
/// Quando o refcount chega a 0, o box é liberado individualmente da root
/// arena via `kata_rt_arena_dealloc`. O `n_captures` no header (offset 16)
/// permite calcular o `size` para o `dealloc`.
///
/// Se o box não está na root arena (arena Bump), `kata_rt_arena_dealloc`
/// é no-op — o lifetime é gerenciado pela arena.
///
/// # Safety
/// `box_ptr` deve ser um ponteiro válido retornado por `kata_rt_alloc_arc`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_decref(box_ptr: i64) -> i64 {
    if box_ptr == 0 {
        return 0;
    }
    unsafe {
        let ptr = box_ptr as *mut u8;
        let refcount_ptr = ptr.add(8) as *mut i64;
        let count = std::ptr::read_unaligned(refcount_ptr);
        if count > 0 {
            let new_count = count - 1;
            std::ptr::write_unaligned(refcount_ptr, new_count);
            if new_count == 0 {
                // ARC chegou a 0 — liberar o bloco da root arena.
                // Lê n_captures do header (offset 16) para calcular o size.
                let n_captures = std::ptr::read_unaligned(ptr.add(N_CAPTURES_OFFSET) as *mut i64);
                let size = HEADER_SIZE as i64 + n_captures * 8;
                let root_arena = kata_rt_get_root_arena_handle();
                if root_arena != 0 {
                    crate::arena::kata_rt_arena_dealloc(root_arena, box_ptr, size);
                }
            }
        }
    }
    0
}

/// Extrai o fn_ptr de um CaptureBox (lê os primeiros 8 bytes).
///
/// # Safety
/// `box_ptr` deve ser um ponteiro válido retornado por `kata_rt_alloc_arc`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arc_fn_ptr(box_ptr: i64) -> i64 {
    if box_ptr == 0 {
        return 0;
    }
    unsafe { std::ptr::read_unaligned(box_ptr as *const i64) }
}