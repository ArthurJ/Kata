//! Arc<T> — CaptureBox para closures com captura.
//!
//! `kata_rt_alloc_arc(fn_ptr, captures_ptr, n_captures)` aloca um CaptureBox
//! na arena global. O box contém:
//!
//! ```text
//! offset 0:  fn_ptr (i64) — ponteiro da função JIT
//! offset 8:  refcount (i64) — contagem de referências (sempre ≥1)
//! offset 16: captures[0] (i64)
//! offset 24: captures[1] (i64)
//! ...
//! offset 16 + (n-1)*8: captures[n-1]
//! ```
//!
//! `kata_rt_incref(box_ptr)` incrementa o refcount.
//! `kata_rt_decref(box_ptr)` decrementa o refcount. Quando chega a 0,
//! o box seria liberado — mas como a arena gerencia o lifetime, o
//! decremento é registrado sem liberar a memória agora. A liberação
//! real acontece no `arena_destroy` (Fase 15 ativará free real).
//!
//! Layout: 16 bytes de header + n_captures * 8 bytes de captures data.

/// Aloca um CaptureBox na arena especificada e retorna o ponteiro.
///
/// Pré-11: `arena_handle` substitui o handle 0 hardcoded.
///
/// `fn_ptr` é o ponteiro da função JIT (para `call_indirect`).
/// `captures_ptr` é um ponteiro para um array de i64 com os valores capturados.
/// `n_captures` é o número de valores capturados.
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

    // Tamanho: 16 bytes header + n_captures * 8 bytes
    let total_size = 16 + n_captures * 8;

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

        // Captures: copia do array de origem para o box
        if n_captures > 0 && captures_ptr != 0 {
            let src = captures_ptr as *const i64;
            let dst = ptr.add(16) as *mut i64;
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
/// Quando o refcount chega a 0, o box seria liberado. Por ora (sem TRMA),
/// a arena gerencia o lifetime — o decremento é registrado mas a memória
/// não é liberada explicitamente. Fase 15 ativará a liberação real.
///
/// # Safety
/// `box_ptr` deve ser um ponteiro válido retornado por `kata_rt_alloc_arc`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_decref(box_ptr: i64) -> i64 {
    if box_ptr == 0 {
        return 0;
    }
    unsafe {
        let refcount_ptr = (box_ptr as *mut u8).add(8) as *mut i64;
        let count = std::ptr::read_unaligned(refcount_ptr);
        if count > 0 {
            std::ptr::write_unaligned(refcount_ptr, count - 1);
        }
        // Fase 15: se count == 1 (vai para 0), liberar o box.
        // Por ora, a arena cuida do lifetime.
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
