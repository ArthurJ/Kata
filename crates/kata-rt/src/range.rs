//! Range — struct alocado na arena com start, step, end.
//!
//! Layout (24 bytes):
//! ```text
//! offset 0:  start (i64)
//! offset 8:  step  (i64)
//! offset 16: end   (i64)
//! ```
//!
//! As operações de next (+) e done (>= ou >) são inlined pelo codegen
//! com base no tipo concreto de A. O runtime só aloca.

/// Aloca 24 bytes na arena para um Range. O codegen faz store de
/// start/step/end via `store` direto nos offsets 0/8/16.
///
/// # Safety
/// `arena_handle` é um handle válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_range_alloc(arena_handle: i64) -> i64 {
    crate::arena::kata_rt_arena_alloc(arena_handle, 24)
}
