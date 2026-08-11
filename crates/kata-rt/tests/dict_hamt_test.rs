use kata_rt::{
    Runtime, kata_rt_arena_create, kata_rt_dict_contains, kata_rt_dict_empty,
    kata_rt_dict_get_checked, kata_rt_dict_insert, kata_rt_dict_len, kata_rt_dict_next,
    kata_rt_dict_remove, kata_rt_hash_int,
};

/// Cria um Runtime para o teste e retorna o ponteiro `i64` a ser passado
/// como primeiro argumento (`rt`) às FFIs migradas para a A2.
fn make_rt() -> i64 {
    let rt = Box::new(Runtime::new());
    let ptr = Box::into_raw(rt) as i64;
    // FFIs periféricas (dict) usam o cache TLS RT_PTR.
    kata_rt::set_rt_ptr(ptr);
    ptr
}

/// Descarta o Runtime criado por `make_rt`.
fn drop_rt(rt_ptr: i64) {
    unsafe { drop(Box::from_raw(rt_ptr as *mut Runtime)) };
}

/// Simple SMI equality for testing: bit-equal comparison of i64.
extern "C" fn smi_eq(a: i64, b: i64) -> i64 {
    if a == b { 1 } else { 0 }
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
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let d = kata_rt_dict_empty(arena);
    assert_eq!(kata_rt_dict_len(d), make_smi(0));
    drop_rt(rt);
}

#[test]
fn dict_insert_then_get() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let d = kata_rt_dict_empty(arena);
    let key = make_smi(42);
    let val = make_smi(100);
    let hash = kata_rt_hash_int(key);
    let eq = fn_ptr_as_i64(smi_eq);

    let d2 = kata_rt_dict_insert(d, key, val, hash, eq, arena);
    assert_eq!(kata_rt_dict_len(d2), make_smi(1));

    let result = kata_rt_dict_get_checked(d2, key, hash, eq, arena);
    assert_eq!(result_tag(result), 0, "key should be found");
    assert_eq!(result_payload(result), val);
    drop_rt(rt);
}

#[test]
fn dict_insert_multiple_keys() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let mut d = kata_rt_dict_empty(arena);
    let eq = fn_ptr_as_i64(smi_eq);

    for i in 1..=100i64 {
        let key = make_smi(i);
        let val = make_smi(i * 10);
        let hash = kata_rt_hash_int(key);
        d = kata_rt_dict_insert(d, key, val, hash, eq, arena);
    }
    assert_eq!(kata_rt_dict_len(d), make_smi(100));

    // Verify a few
    for i in [1i64, 50, 100, 42, 77] {
        let key = make_smi(i);
        let hash = kata_rt_hash_int(key);
        let result = kata_rt_dict_get_checked(d, key, hash, eq, arena);
        assert_eq!(result_tag(result), 0, "key {} should be found", i);
        assert_eq!(result_payload(result), make_smi(i * 10));
    }
    drop_rt(rt);
}

#[test]
fn dict_original_unchanged_after_insert() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let d = kata_rt_dict_empty(arena);
    let key = make_smi(1);
    let hash = kata_rt_hash_int(key);
    let eq = fn_ptr_as_i64(smi_eq);

    let d2 = kata_rt_dict_insert(d, key, make_smi(10), hash, eq, arena);

    // Original should still be empty
    assert_eq!(kata_rt_dict_len(d), make_smi(0));
    assert_eq!(kata_rt_dict_len(d2), make_smi(1));
    drop_rt(rt);
}

#[test]
fn dict_replace_value() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let d = kata_rt_dict_empty(arena);
    let key = make_smi(1);
    let hash = kata_rt_hash_int(key);
    let eq = fn_ptr_as_i64(smi_eq);

    let d1 = kata_rt_dict_insert(d, key, make_smi(10), hash, eq, arena);
    let d2 = kata_rt_dict_insert(d1, key, make_smi(20), hash, eq, arena);
    assert_eq!(kata_rt_dict_len(d2), make_smi(1));

    let result = kata_rt_dict_get_checked(d2, key, hash, eq, arena);
    assert_eq!(result_tag(result), 0);
    assert_eq!(
        result_payload(result),
        make_smi(20),
        "value should be replaced"
    );
    drop_rt(rt);
}

#[test]
fn dict_contains() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
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
    drop_rt(rt);
}

#[test]
fn dict_get_missing_key_returns_err() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let d = kata_rt_dict_empty(arena);
    let key = make_smi(999);
    let hash = kata_rt_hash_int(key);
    let eq = fn_ptr_as_i64(smi_eq);

    let result = kata_rt_dict_get_checked(d, key, hash, eq, arena);
    assert_eq!(result_tag(result), 1, "missing key should return Err");
    drop_rt(rt);
}

#[test]
fn dict_iteration_order() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let mut d = kata_rt_dict_empty(arena);
    let eq = fn_ptr_as_i64(smi_eq);

    // Insert keys 1, 2, 3 in order
    for i in &[1i64, 2, 3] {
        let key = make_smi(*i);
        let val = make_smi(*i * 10);
        let hash = kata_rt_hash_int(key);
        d = kata_rt_dict_insert(d, key, val, hash, eq, arena);
    }

    assert_eq!(kata_rt_dict_len(d), make_smi(3));

    // Iterate via dict_next — should be newest first (3, 2, 1) due to Cons prepend
    let mut keys = Vec::new();
    let mut state = 0i64;
    loop {
        let result = kata_rt_dict_next(d, state, arena);
        let tag = result_tag(result);
        if tag == 1 {
            break; // None — exhausted
        }
        let tuple_ptr = result_payload(result);
        let key = unsafe { std::ptr::read_unaligned(tuple_ptr as *const i64) };
        keys.push(key);
        state += 1;
    }

    // Verify iteration order is 3, 2, 1 (reverse insertion, because Cons prepend)
    assert_eq!(keys.len(), 3, "should have 3 entries");
    assert_eq!(keys[0], make_smi(3), "first should be key 3 (newest)");
    assert_eq!(keys[1], make_smi(2), "second should be key 2");
    assert_eq!(keys[2], make_smi(1), "third should be key 1 (oldest)");
    drop_rt(rt);
}

#[test]
fn dict_iteration_dedup_on_replace() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let mut d = kata_rt_dict_empty(arena);
    let eq = fn_ptr_as_i64(smi_eq);

    let key = make_smi(1);
    let hash = kata_rt_hash_int(key);

    // Insert key=1 with value=10, then replace with value=20
    d = kata_rt_dict_insert(d, key, make_smi(10), hash, eq, arena);
    d = kata_rt_dict_insert(d, key, make_smi(20), hash, eq, arena);

    assert_eq!(
        kata_rt_dict_len(d),
        make_smi(1),
        "should have 1 entry after replace"
    );

    // Iterate — should only see key=1 once (dedup)
    let mut count = 0i64;
    let mut state = 0i64;
    loop {
        let result = kata_rt_dict_next(d, state, arena);
        if result_tag(result) == 1 {
            break;
        }
        let tuple_ptr = result_payload(result);
        let key = unsafe { std::ptr::read_unaligned(tuple_ptr as *const i64) };
        let value =
            unsafe { std::ptr::read_unaligned((tuple_ptr as *const u8).add(8) as *const i64) };
        assert_eq!(key, make_smi(1));
        assert_eq!(value, make_smi(20), "should see the latest value");
        count += 1;
        state += 1;
    }
    assert_eq!(count, 1, "should only see key=1 once (dedup)");
    drop_rt(rt);
}

#[test]
fn dict_iteration_skips_removed() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let mut d = kata_rt_dict_empty(arena);
    let eq = fn_ptr_as_i64(smi_eq);

    // Insert keys 1, 2, 3
    for i in &[1i64, 2, 3] {
        let key = make_smi(*i);
        let hash = kata_rt_hash_int(key);
        d = kata_rt_dict_insert(d, key, make_smi(*i * 10), hash, eq, arena);
    }

    // Remove key 2
    let key2 = make_smi(2);
    let hash2 = kata_rt_hash_int(key2);
    d = kata_rt_dict_remove(d, key2, hash2, eq, arena);

    assert_eq!(
        kata_rt_dict_len(d),
        make_smi(2),
        "should have 2 entries after remove"
    );

    // Iterate — should skip key 2, only see 3 and 1
    let mut keys = Vec::new();
    let mut state = 0i64;
    loop {
        let result = kata_rt_dict_next(d, state, arena);
        if result_tag(result) == 1 {
            break;
        }
        let tuple_ptr = result_payload(result);
        let key = unsafe { std::ptr::read_unaligned(tuple_ptr as *const i64) };
        keys.push(key);
        state += 1;
    }

    assert_eq!(keys.len(), 2, "should have 2 entries (key 2 removed)");
    assert_eq!(keys[0], make_smi(3), "first should be key 3");
    assert_eq!(keys[1], make_smi(1), "second should be key 1");
    // Key 2 should NOT appear
    for k in &keys {
        assert_ne!(*k, make_smi(2), "key 2 should not appear in iteration");
    }
    drop_rt(rt);
}
