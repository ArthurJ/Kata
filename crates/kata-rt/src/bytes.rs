//! Bytes — sequência contígua de u8.
//!
//! Layout (8 + len bytes):
//! ```text
//! offset 0:       len (i64) — número de bytes
//! offset 8+i:     data[i] (u8) — i-ésimo byte
//! ```
//!
//! Imutável: `kata_rt_bytes_set` existe para uso interno (construção de blobs
//! pelo runtime), mas não é exposta na linguagem.

use std::ffi::CString;

/// SMI tag para Byte/Int: (val << 1) | 1.
pub(crate) fn tag_smi(val: i64) -> i64 {
    (val << 1) | 1
}

/// Untag SMI: val >> 1 (descarta bit de tag).
pub(crate) fn untag_smi(val: i64) -> i64 {
    val >> 1
}

/// Aloca um blob de `len` bytes na arena especificada.
/// O blob é zerado. Use `kata_rt_bytes_set` para preencher.
///
/// # Safety
/// `len` deve ser >= 0. `arena_handle` é um handle válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bytes_alloc(len: i64, arena_handle: i64) -> i64 {
    if len < 0 {
        return 0;
    }
    let size = 8 + len; // header (8 bytes) + data (len bytes)
    let ptr = crate::arena::kata_rt_arena_alloc(arena_handle, size);
    if ptr == 0 {
        return 0;
    }
    // Store len no offset 0.
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, len);
        // Zera os bytes de dados (offset 8 até 8+len).
        if len > 0 {
            std::ptr::write_bytes((ptr as *mut u8).add(8), 0, len as usize);
        }
    }
    ptr
}

/// Cria um blob copiando `len` bytes a partir de `src` (ponteiro para dados crus).
/// Retorna ponteiro para o novo blob.
///
/// # Safety
/// `src` deve ser um ponteiro válido com pelo menos `len` bytes legíveis.
/// `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_from_ptr(src: i64, len: i64, arena_handle: i64) -> i64 {
    if src == 0 || len <= 0 {
        // Aloca blob vazio (len=0).
        return kata_rt_bytes_alloc(0, arena_handle);
    }
    let ptr = kata_rt_bytes_alloc(len, arena_handle);
    if ptr == 0 {
        return 0;
    }
    // Copia os bytes de src para o blob (offset 8).
    unsafe {
        std::ptr::copy_nonoverlapping(src as *const u8, (ptr as *mut u8).add(8), len as usize);
    }
    ptr
}

/// Cria um blob a partir de um array de i64s (cada i64 → 1 byte truncado).
/// `count` é o número de elementos no array.
///
/// # Safety
/// `ptrs` deve ser um ponteiro válido para `count` i64s.
/// `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_from_ints(ptrs: i64, count: i64, arena_handle: i64) -> i64 {
    if ptrs == 0 || count <= 0 {
        return kata_rt_bytes_alloc(0, arena_handle);
    }
    let blob = kata_rt_bytes_alloc(count, arena_handle);
    if blob == 0 {
        return 0;
    }
    for i in 0..count {
        let val = unsafe { std::ptr::read_unaligned((ptrs as *const i64).add(i as usize)) };
        let byte = untag_smi(val) as u8;
        unsafe {
            std::ptr::write_unaligned((blob as *mut u8).add(8 + i as usize), byte);
        }
    }
    blob
}

/// Retorna o número de bytes do blob (load offset 0). SMI-tagged.
///
/// # Safety
/// `ptr` deve ser um ponteiro válido retornado por `kata_rt_bytes_alloc`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bytes_len(ptr: i64) -> i64 {
    if ptr == 0 {
        return 1; // SMI(0) = (0 << 1) | 1 = 1
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    tag_smi(len)
}

/// Retorna o byte no índice `idx` como SMI-tagged. Sem bounds check.
///
/// # Safety
/// `ptr` deve ser válido e `idx` deve estar em [0, len).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bytes_get(ptr: i64, idx: i64) -> i64 {
    if ptr == 0 {
        return tag_smi(0);
    }
    let byte = unsafe { std::ptr::read_unaligned((ptr as *const u8).add(8 + idx as usize)) };
    tag_smi(byte as i64)
}

/// Armazena `val` (SMI-tagged) no índice `idx`. Sem bounds check.
/// Uso interno apenas — não exposta na linguagem.
///
/// # Safety
/// `ptr` deve ser válido e `idx` deve estar em [0, len).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bytes_set(ptr: i64, idx: i64, val: i64) {
    if ptr == 0 {
        return;
    }
    let byte = untag_smi(val) as u8;
    unsafe {
        std::ptr::write_unaligned((ptr as *mut u8).add(8 + idx as usize), byte);
    }
}

/// Acesso por índice com bounds check. Retorna um Result box (Sum):
/// - Ok: tag=0, payload=byte (SMI-tagged)
/// - Err: tag=1, payload=0 (out of bounds)
///
/// Layout do Result box: igual a kata_rt_store_sum_result (16 bytes).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bytes_get_checked(ptr: i64, idx: i64) -> i64 {
    if ptr == 0 {
        return crate::sum::kata_rt_store_sum_result(1, 0, 0);
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    // Suporte a índice negativo (do final).
    let real_idx = if idx < 0 { len + idx } else { idx };
    if real_idx < 0 || real_idx >= len {
        return crate::sum::kata_rt_store_sum_result(1, 0, 0);
    }
    let byte = unsafe { std::ptr::read_unaligned((ptr as *const u8).add(8 + real_idx as usize)) };
    crate::sum::kata_rt_store_sum_result(0, tag_smi(byte as i64), 0)
}

/// Concatena dois blobs. Aloca novo blob com len_a + len_b bytes.
///
/// # Safety
/// `a` e `b` devem ser ponteiros válidos (ou 0 para blob vazio).
/// `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_concat(a: i64, b: i64, arena_handle: i64) -> i64 {
    let len_a = if a == 0 {
        0
    } else {
        unsafe { std::ptr::read_unaligned(a as *const i64) }
    };
    let len_b = if b == 0 {
        0
    } else {
        unsafe { std::ptr::read_unaligned(b as *const i64) }
    };
    let total = len_a + len_b;
    let ptr = kata_rt_bytes_alloc(total, arena_handle);
    if ptr == 0 {
        return 0;
    }
    // Copia bytes de a.
    if len_a > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(
                (a as *const u8).add(8),
                (ptr as *mut u8).add(8),
                len_a as usize,
            );
        }
    }
    // Copia bytes de b.
    if len_b > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(
                (b as *const u8).add(8),
                (ptr as *mut u8).add(8 + len_a as usize),
                len_b as usize,
            );
        }
    }
    ptr
}

/// Compara dois blobs byte-a-byte. Retorna 1 se iguais, 0 se diferentes.
///
/// # Safety
/// `a` e `b` devem ser ponteiros válidos (ou 0 para blob vazio).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_eq(a: i64, b: i64) -> i64 {
    if a == 0 && b == 0 {
        return 1;
    }
    if a == 0 || b == 0 {
        return 0;
    }
    let len_a = unsafe { std::ptr::read_unaligned(a as *const i64) };
    let len_b = unsafe { std::ptr::read_unaligned(b as *const i64) };
    if len_a != len_b {
        return 0;
    }
    let bytes_a = unsafe { std::slice::from_raw_parts((a as *const u8).add(8), len_a as usize) };
    let bytes_b = unsafe { std::slice::from_raw_parts((b as *const u8).add(8), len_b as usize) };
    if bytes_a == bytes_b { 1 } else { 0 }
}

/// Negação de igualdade entre dois blobs. Retorna 1 se diferentes, 0 se iguais.
///
/// # Safety
/// `a` e `b` devem ser ponteiros válidos (ou 0 para blob vazio).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_neq(a: i64, b: i64) -> i64 {
    1 - unsafe { kata_rt_bytes_eq(a, b) }
}

/// Representação hex do blob como C string. Retorna ponteiro (ownership transferida).
///
/// Exemplo: `b"Hello"` → `"48656c6c6f"`
///
/// # Safety
/// `ptr` deve ser um ponteiro válido (ou 0 para blob vazio).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bytes_show(ptr: i64) -> *mut std::os::raw::c_char {
    if ptr == 0 {
        return CString::new("")
            .expect("CString vazia sempre válida")
            .into_raw();
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    let bytes = if len <= 0 {
        &b""[..]
    } else {
        unsafe { std::slice::from_raw_parts((ptr as *const u8).add(8), len as usize) }
    };
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    CString::new(hex)
        .unwrap_or_else(|_| CString::new("").expect("CString vazia sempre válida"))
        .into_raw()
}

/// Slice — cria sub-blob de start (inclusive) até end (exclusive).
/// Suporta índices negativos (do final).
///
/// # Safety
/// `ptr` deve ser válido. `start` e `end` devem estar em [-len, len].
/// `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_slice(
    ptr: i64,
    start: i64,
    end: i64,
    arena_handle: i64,
) -> i64 {
    if ptr == 0 {
        return kata_rt_bytes_alloc(0, arena_handle);
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    // Normaliza índices negativos.
    let start = if start < 0 { len + start } else { start };
    let end = if end < 0 { len + end } else { end };
    // Clamp.
    let start = start.clamp(0, len);
    let end = end.clamp(0, len);
    if start >= end {
        return kata_rt_bytes_alloc(0, arena_handle);
    }
    let sub_len = end - start;
    let new_ptr = kata_rt_bytes_alloc(sub_len, arena_handle);
    if new_ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(
            (ptr as *const u8).add(8 + start as usize),
            (new_ptr as *mut u8).add(8),
            sub_len as usize,
        );
    }
    new_ptr
}

// ── Operações bitwise ──────────────────────────────────────

/// AND bit-a-bit de dois blobs (elemento-a-elemento). Requer mesmo tamanho.
///
/// # Safety
/// `a` e `b` devem ser válidos e de mesmo comprimento.
/// `arena_handle` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_and(a: i64, b: i64, arena_handle: i64) -> i64 {
    bitwise_elementwise(a, b, arena_handle, |x, y| x & y)
}

/// OR bit-a-bit de dois blobs (elemento-a-elemento).
///
/// # Safety
/// `a` e `b` devem ser válidos e de mesmo comprimento.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_or(a: i64, b: i64, arena_handle: i64) -> i64 {
    bitwise_elementwise(a, b, arena_handle, |x, y| x | y)
}

/// XOR bit-a-bit de dois blobs (elemento-a-elemento).
///
/// # Safety
/// `a` e `b` devem ser válidos e de mesmo comprimento.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_xor(a: i64, b: i64, arena_handle: i64) -> i64 {
    bitwise_elementwise(a, b, arena_handle, |x, y| x ^ y)
}

/// NOT bit-a-bit (inverte todos os bits de cada byte).
///
/// # Safety
/// `ptr` deve ser válido.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bytes_not(ptr: i64, arena_handle: i64) -> i64 {
    if ptr == 0 {
        return kata_rt_bytes_alloc(0, arena_handle);
    }
    let len = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
    let new_ptr = kata_rt_bytes_alloc(len, arena_handle);
    if new_ptr == 0 {
        return 0;
    }
    for i in 0..len {
        let byte = unsafe { std::ptr::read_unaligned((ptr as *const u8).add(8 + i as usize)) };
        unsafe {
            std::ptr::write_unaligned((new_ptr as *mut u8).add(8 + i as usize), !byte);
        }
    }
    new_ptr
}

/// Aplica uma operação bitwise elemento-a-elemento entre dois blobs.
/// Requer que ambos tenham o mesmo comprimento.
fn bitwise_elementwise<F>(a: i64, b: i64, arena_handle: i64, op: F) -> i64
where
    F: Fn(u8, u8) -> u8,
{
    if a == 0 || b == 0 {
        return kata_rt_bytes_alloc(0, arena_handle);
    }
    let len_a = unsafe { std::ptr::read_unaligned(a as *const i64) };
    let len_b = unsafe { std::ptr::read_unaligned(b as *const i64) };
    if len_a != len_b {
        // Tamanhos diferentes — retorna blob vazio (erro em runtime).
        return kata_rt_bytes_alloc(0, arena_handle);
    }
    let new_ptr = kata_rt_bytes_alloc(len_a, arena_handle);
    if new_ptr == 0 {
        return 0;
    }
    for i in 0..len_a {
        let byte_a = unsafe { std::ptr::read_unaligned((a as *const u8).add(8 + i as usize)) };
        let byte_b = unsafe { std::ptr::read_unaligned((b as *const u8).add(8 + i as usize)) };
        let result = op(byte_a, byte_b);
        unsafe {
            std::ptr::write_unaligned((new_ptr as *mut u8).add(8 + i as usize), result);
        }
    }
    new_ptr
}

// ── Operações bitwise escalares (Byte) ─────────────────────
// Movidas para byte.rs (módulo dedicado a operações escalares Byte).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::kata_rt_arena_create;

    fn make_arena() -> i64 {
        kata_rt_arena_create()
    }

    #[test]
    fn alloc_and_len() {
        let arena = make_arena();
        let ptr = kata_rt_bytes_alloc(5, arena);
        assert!(ptr != 0);
        assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 5);
    }

    #[test]
    fn alloc_zero_len() {
        let arena = make_arena();
        let ptr = kata_rt_bytes_alloc(0, arena);
        assert!(ptr != 0);
        assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 0);
    }

    #[test]
    fn alloc_negative_returns_zero() {
        let arena = make_arena();
        assert_eq!(kata_rt_bytes_alloc(-1, arena), 0);
    }

    #[test]
    fn set_and_get() {
        let arena = make_arena();
        let ptr = kata_rt_bytes_alloc(3, arena);
        kata_rt_bytes_set(ptr, 0, tag_smi(0x41));
        kata_rt_bytes_set(ptr, 1, tag_smi(0x42));
        kata_rt_bytes_set(ptr, 2, tag_smi(0x43));
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 0)), 0x41);
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 1)), 0x42);
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 2)), 0x43);
    }

    #[test]
    fn get_checked_in_bounds() {
        let arena = make_arena();
        let ptr = kata_rt_bytes_alloc(2, arena);
        kata_rt_bytes_set(ptr, 0, tag_smi(0xFF));
        let result = kata_rt_bytes_get_checked(ptr, 0);
        let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
        let payload =
            unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
        assert_eq!(tag, 0); // Ok
        assert_eq!(untag_smi(payload), 0xFF);
    }

    #[test]
    fn get_checked_out_of_bounds() {
        let arena = make_arena();
        let ptr = kata_rt_bytes_alloc(2, arena);
        let result = kata_rt_bytes_get_checked(ptr, 5);
        let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
        assert_eq!(tag, 1); // Err
    }

    #[test]
    fn get_checked_negative_index() {
        let arena = make_arena();
        let ptr = kata_rt_bytes_alloc(3, arena);
        kata_rt_bytes_set(ptr, 2, tag_smi(0x5A));
        let result = kata_rt_bytes_get_checked(ptr, -1);
        let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
        let payload =
            unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
        assert_eq!(tag, 0); // Ok
        assert_eq!(untag_smi(payload), 0x5A);
    }

    #[test]
    fn from_ptr() {
        let arena = make_arena();
        let data = [0x48u8, 0x65, 0x6C, 0x6C, 0x6F]; // "Hello"
        let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 5, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 5);
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 0)), 0x48); // 'H'
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 4)), 0x6F); // 'o'
    }

    #[test]
    fn from_ints() {
        let arena = make_arena();
        let ints = [tag_smi(0x41), tag_smi(0x42), tag_smi(0x43)];
        let ptr = unsafe { kata_rt_bytes_from_ints(ints.as_ptr() as i64, 3, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 3);
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 0)), 0x41);
        assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 2)), 0x43);
    }

    #[test]
    fn concat() {
        let arena = make_arena();
        let a = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x42].as_ptr() as i64, 2, arena) };
        let b = unsafe { kata_rt_bytes_from_ptr([0x43u8, 0x44, 0x45].as_ptr() as i64, 3, arena) };
        let c = unsafe { kata_rt_bytes_concat(a, b, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_len(c)), 5);
        assert_eq!(untag_smi(kata_rt_bytes_get(c, 0)), 0x41);
        assert_eq!(untag_smi(kata_rt_bytes_get(c, 1)), 0x42);
        assert_eq!(untag_smi(kata_rt_bytes_get(c, 2)), 0x43);
        assert_eq!(untag_smi(kata_rt_bytes_get(c, 4)), 0x45);
    }

    #[test]
    fn concat_with_empty() {
        let arena = make_arena();
        let a = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x42].as_ptr() as i64, 2, arena) };
        let empty = kata_rt_bytes_alloc(0, arena);
        let c = unsafe { kata_rt_bytes_concat(a, empty, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_len(c)), 2);
        assert_eq!(untag_smi(kata_rt_bytes_get(c, 0)), 0x41);
    }

    #[test]
    fn eq() {
        let arena = make_arena();
        let a = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x42].as_ptr() as i64, 2, arena) };
        let b = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x42].as_ptr() as i64, 2, arena) };
        let c = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x43].as_ptr() as i64, 2, arena) };
        assert_eq!(unsafe { kata_rt_bytes_eq(a, b) }, 1);
        assert_eq!(unsafe { kata_rt_bytes_eq(a, c) }, 0);
    }

    #[test]
    fn show_hex() {
        let arena = make_arena();
        let data = [0x48u8, 0x65, 0x6C, 0x6C, 0x6F]; // "Hello"
        let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 5, arena) };
        let result = kata_rt_bytes_show(ptr);
        let s = unsafe { std::ffi::CStr::from_ptr(result).to_str().unwrap() };
        assert_eq!(s, "48656c6c6f");
        unsafe { _ = CString::from_raw(result) };
    }

    #[test]
    fn slice_basic() {
        let arena = make_arena();
        let data = [0x41u8, 0x42, 0x43, 0x44, 0x45];
        let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 5, arena) };
        let sub = unsafe { kata_rt_bytes_slice(ptr, 1, 3, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_len(sub)), 2);
        assert_eq!(untag_smi(kata_rt_bytes_get(sub, 0)), 0x42);
        assert_eq!(untag_smi(kata_rt_bytes_get(sub, 1)), 0x43);
    }

    #[test]
    fn slice_negative_index() {
        let arena = make_arena();
        let data = [0x41u8, 0x42, 0x43, 0x44, 0x45];
        let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 5, arena) };
        let sub = unsafe { kata_rt_bytes_slice(ptr, -2, 5, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_len(sub)), 2);
        assert_eq!(untag_smi(kata_rt_bytes_get(sub, 0)), 0x44);
        assert_eq!(untag_smi(kata_rt_bytes_get(sub, 1)), 0x45);
    }

    #[test]
    fn slice_empty() {
        let arena = make_arena();
        let data = [0x41u8, 0x42];
        let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 2, arena) };
        let sub = unsafe { kata_rt_bytes_slice(ptr, 1, 1, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_len(sub)), 0);
    }

    #[test]
    fn bitwise_and() {
        let arena = make_arena();
        let a = unsafe { kata_rt_bytes_from_ptr([0xFFu8, 0xF0, 0x0F].as_ptr() as i64, 3, arena) };
        let b = unsafe { kata_rt_bytes_from_ptr([0xAAu8, 0xFF, 0x0A].as_ptr() as i64, 3, arena) };
        let result = unsafe { kata_rt_bytes_and(a, b, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 0)), 0xAA);
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 1)), 0xF0);
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 2)), 0x0A);
    }

    #[test]
    fn bitwise_or() {
        let arena = make_arena();
        let a = unsafe { kata_rt_bytes_from_ptr([0xF0u8, 0x0F].as_ptr() as i64, 2, arena) };
        let b = unsafe { kata_rt_bytes_from_ptr([0x0Fu8, 0xF0].as_ptr() as i64, 2, arena) };
        let result = unsafe { kata_rt_bytes_or(a, b, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 0)), 0xFF);
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 1)), 0xFF);
    }

    #[test]
    fn bitwise_xor() {
        let arena = make_arena();
        let a = unsafe { kata_rt_bytes_from_ptr([0xFFu8, 0x00].as_ptr() as i64, 2, arena) };
        let b = unsafe { kata_rt_bytes_from_ptr([0x0Fu8, 0x00].as_ptr() as i64, 2, arena) };
        let result = unsafe { kata_rt_bytes_xor(a, b, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 0)), 0xF0);
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 1)), 0x00);
    }

    #[test]
    fn bitwise_not() {
        let arena = make_arena();
        let a = unsafe { kata_rt_bytes_from_ptr([0xF0u8, 0x0F].as_ptr() as i64, 2, arena) };
        let result = unsafe { kata_rt_bytes_not(a, arena) };
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 0)), 0x0F);
        assert_eq!(untag_smi(kata_rt_bytes_get(result, 1)), 0xF0);
    }
}
