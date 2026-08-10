//! Range — struct alocado na arena com start, step, end, inclusive.
//!
//! Layout (32 bytes):
//! ```text
//! offset 0:  start     (i64)
//! offset 8:  step      (i64)
//! offset 16: end       (i64)
//! offset 24: inclusive  (i64, SMI: 1 = inclusive, 0 = exclusive)
//! ```
//!
//! As operações de next (+) e done (>= ou >) são inlined pelo codegen
//! com base no tipo concreto de A e no sinal do step. O runtime só aloca.

/// Aloca 32 bytes na arena para um Range. O codegen faz store de
/// start/step/end/inclusive via `store` direto nos offsets 0/8/16/24.
///
/// # Safety
/// `arena_handle` é um handle válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_range_alloc(arena_handle: i64) -> i64 {
    crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 32)
}
