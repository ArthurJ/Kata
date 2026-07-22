use kata_rt::{
    kata_rt_arena_create, kata_rt_dict_contains, kata_rt_dict_empty, kata_rt_dict_get_checked,
    kata_rt_dict_insert, kata_rt_dict_len, kata_rt_hash_int,
};

/// Simple SMI equality for testing: bit-equal comparison of i64.
extern "C" fn smi_eq(a: i64, b: i64) -> i64 {
    if a == b {
        1
    } else {
        0
    }
}

/// Cast function pointer to i64 (avoids `function_casts_as_integer` warning).
fn fn_ptr_as_i64(f: extern "C" fn(i64, i64) -> i64) -> i64 {
    f as *const () as i64
}

fn make_smi(n: i64) -> i64 {
    (n << 1) | 1
}

/// Read the tag from a Result box (offset 0).
fn result_tag(result: i64) -> i64 {
    unsafe { std::ptr::read_unaligned(result as *const i64) }
}

/// Read the payload from a Result box (offset 8).
fn result_payload(result: i64) -> i64 {
    unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) }
}

#[test]
fn dict_empty_has_len_zero() {
    let arena = kata_rt_arena_create();
    let d = kata_rt_dict_empty(arena);
    assert_eq!(kata_rt_dict_len(d), 0);
}

#[test]
fn dict_insert_then_get() {
    let arena = kata_rt_arena_create();
    let d = kata_rt_dict_empty(arena);
    let key = make_smi(42);
    let val = make_smi(100);
    let hash = kata_rt_hash_int(key);
    let eq = fn_ptr_as_i64(smi_eq);

    let d2 = kata_rt_dict_insert(d, key, val, hash, eq, arena);
    assert_eq!(kata_rt_dict_len(d2), 1);

    let result = kata_rt_dict_get_checked(d2, key, hash, eq, arena);
    assert_eq!(result_tag(result), 0, "key should be found");
    assert_eq!(result_payload(result), val);
}

#[test]
fn dict_insert_multiple_keys() {
    let arena = kata_rt_arena_create();
    let mut d = kata_rt_dict_empty(arena);
    let eq = fn_ptr_as_i64(smi_eq);

    for i in 1..=100i64 {
        let key = make_smi(i);
        let val = make_smi(i * 10);
        let hash = kata_rt_hash_int(key);
        d = kata_rt_dict_insert(d, key, val, hash, eq, arena);
    }
    assert_eq!(kata_rt_dict_len(d), 100);

    // Verify a few
    for i in [1i64, 50, 100, 42, 77] {
        let key = make_smi(i);
        let hash = kata_rt_hash_int(key);
        let result = kata_rt_dict_get_checked(d, key, hash, eq, arena);
        assert_eq!(result_tag(result), 0, "key {} should be found", i);
        assert_eq!(result_payload(result), make_smi(i * 10));
    }
}

#[test]
fn dict_original_unchanged_after_insert() {
    let arena = kata_rt_arena_create();
    let d = kata_rt_dict_empty(arena);
    let key = make_smi(1);
    let hash = kata_rt_hash_int(key);
    let eq = fn_ptr_as_i64(smi_eq);

    let d2 = kata_rt_dict_insert(d, key, make_smi(10), hash, eq, arena);

    // Original should still be empty
    assert_eq!(kata_rt_dict_len(d), 0);
    assert_eq!(kata_rt_dict_len(d2), 1);
}

#[test]
fn dict_replace_value() {
    let arena = kata_rt_arena_create();
    let d = kata_rt_dict_empty(arena);
    let key = make_smi(1);
    let hash = kata_rt_hash_int(key);
    let eq = fn_ptr_as_i64(smi_eq);

    let d1 = kata_rt_dict_insert(d, key, make_smi(10), hash, eq, arena);
    let d2 = kata_rt_dict_insert(d1, key, make_smi(20), hash, eq, arena);
    assert_eq!(kata_rt_dict_len(d2), 1);

    let result = kata_rt_dict_get_checked(d2, key, hash, eq, arena);
    assert_eq!(result_tag(result), 0);
    assert_eq!(
        result_payload(result),
        make_smi(20),
        "value should be replaced"
    );
}

#[test]
fn dict_contains() {
    let arena = kata_rt_arena_create();
    let mut d = kata_rt_dict_empty(arena);
    let eq = fn_ptr_as_i64(smi_eq);

    for i in 1..=50i64 {
        let key = make_smi(i);
        let val = make_smi(i);
        let hash = kata_rt_hash_int(key);
        d = kata_rt_dict_insert(d, key, val, hash, eq, arena);
    }

    // Present keys
    for i in [1i64, 25, 50] {
        let key = make_smi(i);
        let hash = kata_rt_hash_int(key);
        assert_eq!(
            kata_rt_dict_contains(d, key, hash, eq),
            1,
            "key {} should be contained",
            i
        );
    }

    // Absent keys
    for i in [51i64, 100, 0] {
        let key = make_smi(i);
        let hash = kata_rt_hash_int(key);
        assert_eq!(
            kata_rt_dict_contains(d, key, hash, eq),
            0,
            "key {} should NOT be contained",
            i
        );
    }
}

#[test]
fn dict_get_missing_key_returns_err() {
    let arena = kata_rt_arena_create();
    let d = kata_rt_dict_empty(arena);
    let key = make_smi(999);
    let hash = kata_rt_hash_int(key);
    let eq = fn_ptr_as_i64(smi_eq);

    let result = kata_rt_dict_get_checked(d, key, hash, eq, arena);
    assert_eq!(result_tag(result), 1, "missing key should return Err");
}