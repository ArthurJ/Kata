use super::*;
use crate::arena::kata_rt_arena_create;

/// Guard que cria um Runtime, seta em TLS, e limpa no Drop.
/// A arena vive dentro do Runtime, então o Runtime deve sobreviver ao teste todo.
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

/// Cria runtime + arena. Retorna (guard, arena_handle).
/// O guard deve ser mantido vivo até o fim do teste.
fn make_arena() -> (TestRt, i64) {
    let rt = TestRt::new();
    let arena = kata_rt_arena_create(rt.rt_ptr);
    (rt, arena)
}

#[test]
fn alloc_and_len() {
    let (_rt, arena) = make_arena();
    let ptr = kata_rt_bytes_alloc(5, arena);
    assert!(ptr != 0);
    assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 5);
}

#[test]
fn alloc_zero_len() {
    let (_rt, arena) = make_arena();
    let ptr = kata_rt_bytes_alloc(0, arena);
    assert!(ptr != 0);
    assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 0);
}

#[test]
fn alloc_negative_returns_zero() {
    let (_rt, arena) = make_arena();
    assert_eq!(kata_rt_bytes_alloc(-1, arena), 0);
}

#[test]
fn set_and_get() {
    let (_rt, arena) = make_arena();
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
    let (_rt, arena) = make_arena();
    let ptr = kata_rt_bytes_alloc(2, arena);
    kata_rt_bytes_set(ptr, 0, tag_smi(0xFF));
    let result = kata_rt_bytes_get_checked(ptr, 0);
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    let payload = unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
    assert_eq!(tag, 0); // Ok
    assert_eq!(untag_smi(payload), 0xFF);
}

#[test]
fn get_checked_out_of_bounds() {
    let (_rt, arena) = make_arena();
    let ptr = kata_rt_bytes_alloc(2, arena);
    let result = kata_rt_bytes_get_checked(ptr, 5);
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    assert_eq!(tag, 1); // Err
}

#[test]
fn get_checked_negative_index() {
    let (_rt, arena) = make_arena();
    let ptr = kata_rt_bytes_alloc(3, arena);
    kata_rt_bytes_set(ptr, 2, tag_smi(0x5A));
    let result = kata_rt_bytes_get_checked(ptr, -1);
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    let payload = unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
    assert_eq!(tag, 0); // Ok
    assert_eq!(untag_smi(payload), 0x5A);
}

#[test]
fn from_ptr() {
    let (_rt, arena) = make_arena();
    let data = [0x48u8, 0x65, 0x6C, 0x6C, 0x6F]; // "Hello"
    let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 5, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 5);
    assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 0)), 0x48); // 'H'
    assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 4)), 0x6F); // 'o'
}

#[test]
fn from_ints() {
    let (_rt, arena) = make_arena();
    let ints = [tag_smi(0x41), tag_smi(0x42), tag_smi(0x43)];
    let ptr = unsafe { kata_rt_bytes_from_ints(ints.as_ptr() as i64, 3, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 3);
    assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 0)), 0x41);
    assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 2)), 0x43);
}

#[test]
fn concat() {
    let (_rt, arena) = make_arena();
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
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x42].as_ptr() as i64, 2, arena) };
    let empty = kata_rt_bytes_alloc(0, arena);
    let c = unsafe { kata_rt_bytes_concat(a, empty, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(c)), 2);
    assert_eq!(untag_smi(kata_rt_bytes_get(c, 0)), 0x41);
}

#[test]
fn eq() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x42].as_ptr() as i64, 2, arena) };
    let b = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x42].as_ptr() as i64, 2, arena) };
    let c = unsafe { kata_rt_bytes_from_ptr([0x41u8, 0x43].as_ptr() as i64, 2, arena) };
    assert_eq!(unsafe { kata_rt_bytes_eq(a, b) }, 1);
    assert_eq!(unsafe { kata_rt_bytes_eq(a, c) }, 0);
}

#[test]
fn show_hex() {
    let (_rt, arena) = make_arena();
    let data = [0x48u8, 0x65, 0x6C, 0x6C, 0x6F]; // "Hello"
    let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 5, arena) };
    let result = kata_rt_bytes_show(ptr);
    let s = unsafe { std::ffi::CStr::from_ptr(result).to_str().unwrap() };
    assert_eq!(s, "48656c6c6f");
    unsafe { _ = CString::from_raw(result) };
}

#[test]
fn slice_basic() {
    let (_rt, arena) = make_arena();
    let data = [0x41u8, 0x42, 0x43, 0x44, 0x45];
    let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 5, arena) };
    let sub = unsafe { kata_rt_bytes_slice(ptr, tag_smi(1), tag_smi(3), arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(sub)), 2);
    assert_eq!(untag_smi(kata_rt_bytes_get(sub, 0)), 0x42);
    assert_eq!(untag_smi(kata_rt_bytes_get(sub, 1)), 0x43);
}

#[test]
fn slice_negative_index() {
    let (_rt, arena) = make_arena();
    let data = [0x41u8, 0x42, 0x43, 0x44, 0x45];
    let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 5, arena) };
    let sub = unsafe { kata_rt_bytes_slice(ptr, tag_smi(-2), tag_smi(5), arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(sub)), 2);
    assert_eq!(untag_smi(kata_rt_bytes_get(sub, 0)), 0x44);
    assert_eq!(untag_smi(kata_rt_bytes_get(sub, 1)), 0x45);
}

#[test]
fn slice_empty() {
    let (_rt, arena) = make_arena();
    let data = [0x41u8, 0x42];
    let ptr = unsafe { kata_rt_bytes_from_ptr(data.as_ptr() as i64, 2, arena) };
    let sub = unsafe { kata_rt_bytes_slice(ptr, 1, 1, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(sub)), 0);
}

#[test]
fn bitwise_and() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0xFFu8, 0xF0, 0x0F].as_ptr() as i64, 3, arena) };
    let b = unsafe { kata_rt_bytes_from_ptr([0xAAu8, 0xFF, 0x0A].as_ptr() as i64, 3, arena) };
    let result = unsafe { kata_rt_bytes_and(a, b, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 0)), 0xAA);
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 1)), 0xF0);
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 2)), 0x0A);
}

#[test]
fn bitwise_or() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0xF0u8, 0x0F].as_ptr() as i64, 2, arena) };
    let b = unsafe { kata_rt_bytes_from_ptr([0x0Fu8, 0xF0].as_ptr() as i64, 2, arena) };
    let result = unsafe { kata_rt_bytes_or(a, b, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 0)), 0xFF);
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 1)), 0xFF);
}

#[test]
fn bitwise_xor() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0xFFu8, 0x00].as_ptr() as i64, 2, arena) };
    let b = unsafe { kata_rt_bytes_from_ptr([0x0Fu8, 0x00].as_ptr() as i64, 2, arena) };
    let result = unsafe { kata_rt_bytes_xor(a, b, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 0)), 0xF0);
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 1)), 0x00);
}

#[test]
fn bitwise_not() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0xF0u8, 0x0F].as_ptr() as i64, 2, arena) };
    let result = unsafe { kata_rt_bytes_not(a, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 0)), 0x0F);
    assert_eq!(untag_smi(kata_rt_bytes_get(result, 1)), 0xF0);
}

// ── Broadcast: tamanhos diferentes ──────────────────────────────

/// AND com broadcast: menor é zero-padded. Bytes extras do maior AND 0 = 0.
#[test]
fn bitwise_and_broadcast() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0xFFu8, 0xF0].as_ptr() as i64, 2, arena) };
    let b = unsafe { kata_rt_bytes_from_ptr([0xAAu8].as_ptr() as i64, 1, arena) };
    let result = unsafe { kata_rt_bytes_and(a, b, arena) };
    assert_eq!(
        untag_smi(kata_rt_bytes_len(result)),
        2,
        "resultado tem tamanho do maior"
    );
    assert_eq!(
        untag_smi(kata_rt_bytes_get(result, 0)),
        0xAA,
        "0xFF AND 0xAA = 0xAA"
    );
    assert_eq!(
        untag_smi(kata_rt_bytes_get(result, 1)),
        0x00,
        "0xF0 AND 0 (pad) = 0x00"
    );
}

/// OR com broadcast: bytes extras do maior OR 0 = preserva o byte.
#[test]
fn bitwise_or_broadcast() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0xF0u8, 0x0F].as_ptr() as i64, 2, arena) };
    let b = unsafe { kata_rt_bytes_from_ptr([0x0Fu8].as_ptr() as i64, 1, arena) };
    let result = unsafe { kata_rt_bytes_or(a, b, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(result)), 2);
    assert_eq!(
        untag_smi(kata_rt_bytes_get(result, 0)),
        0xFF,
        "0xF0 OR 0x0F = 0xFF"
    );
    assert_eq!(
        untag_smi(kata_rt_bytes_get(result, 1)),
        0x0F,
        "0x0F OR 0 (pad) = 0x0F"
    );
}

/// XOR com broadcast: bytes extras do maior XOR 0 = preserva o byte.
#[test]
fn bitwise_xor_broadcast() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0xFFu8, 0x42].as_ptr() as i64, 2, arena) };
    let b = unsafe { kata_rt_bytes_from_ptr([0x0Fu8].as_ptr() as i64, 1, arena) };
    let result = unsafe { kata_rt_bytes_xor(a, b, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(result)), 2);
    assert_eq!(
        untag_smi(kata_rt_bytes_get(result, 0)),
        0xF0,
        "0xFF XOR 0x0F = 0xF0"
    );
    assert_eq!(
        untag_smi(kata_rt_bytes_get(result, 1)),
        0x42,
        "0x42 XOR 0 (pad) = 0x42"
    );
}

/// Broadcast com primeiro operand menor.
#[test]
fn bitwise_and_broadcast_first_shorter() {
    let (_rt, arena) = make_arena();
    let a = unsafe { kata_rt_bytes_from_ptr([0xFFu8].as_ptr() as i64, 1, arena) };
    let b = unsafe { kata_rt_bytes_from_ptr([0xAAu8, 0xBB].as_ptr() as i64, 2, arena) };
    let result = unsafe { kata_rt_bytes_and(a, b, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(result)), 2);
    assert_eq!(
        untag_smi(kata_rt_bytes_get(result, 0)),
        0xAA,
        "0xFF AND 0xAA = 0xAA"
    );
    assert_eq!(
        untag_smi(kata_rt_bytes_get(result, 1)),
        0x00,
        "0 (pad) AND 0xBB = 0x00"
    );
}
