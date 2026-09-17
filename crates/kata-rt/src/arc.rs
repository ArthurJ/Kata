//! CaptureBox para closures com captura.
//!
//! `kata_rt_alloc_arc(rt, fn_ptr, captures_ptr, n_captures, arena_handle)` aloca
//! um CaptureBox na arena especificada. O box contém:
//!
//! ```text
//! offset 0:  fn_ptr (i64) — ponteiro da função JIT
//! offset 8:  n_captures (i64) — número de captures
//! offset 16: captures[0] (i64)
//! offset 24: captures[1] (i64)
//! ...
//! offset 16 + (n-1)*8: captures[n-1]
//! ```
//!
//! Sem refcount — arenas Bump não suportam dealloc individual. O box
//! sobrevive até a arena ser resetada (fiber termina) ou destruída (teardown).

/// Offset do `n_captures` no header do CaptureBox.
const N_CAPTURES_OFFSET: usize = 8;
/// Offset do primeiro capture no CaptureBox.
const CAPTURES_OFFSET: usize = 16;
/// Tamanho do header (fn_ptr + n_captures).
const HEADER_SIZE: usize = 16;

/// Aloca um CaptureBox na arena especificada e retorna o ponteiro.
///
/// `rt` é o ponteiro para `Runtime` (necessário para acessar o pool de arenas).
/// `fn_ptr` é o ponteiro da função JIT (para `call_indirect`).
/// `captures_ptr` é um ponteiro para um array de i64 com os valores capturados.
/// `n_captures` é o número de valores capturados.
/// `arena_handle` é o handle da arena onde o box é alocado.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_alloc_arc(
    rt: i64,
    fn_ptr: i64,
    captures_ptr: i64,
    n_captures: i64,
    arena_handle: i64,
) -> i64 {
    if n_captures < 0 {
        return 0;
    }

    let total_size = HEADER_SIZE as i64 + n_captures * 8;
    let box_ptr = crate::arena::kata_rt_arena_alloc(rt, arena_handle, total_size);
    if box_ptr == 0 {
        return 0;
    }

    unsafe {
        let ptr = box_ptr as *mut u8;
        std::ptr::write_unaligned(ptr as *mut i64, fn_ptr);
        std::ptr::write_unaligned(ptr.add(N_CAPTURES_OFFSET) as *mut i64, n_captures);

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
