use kata_rt::{
    kata_rt_arena_create, kata_rt_hash_int, kata_rt_set_contains, kata_rt_set_difference,
    kata_rt_set_empty, kata_rt_set_insert, kata_rt_set_intersection, kata_rt_set_len,
    kata_rt_set_union,
};

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

/// Build a set from a list of SMI-encoded ints.
fn build_set(arena: i64, eq: i64, vals: &[i64]) -> i64 {
    let mut s = kata_rt_set_empty(arena);
    for &v in vals {
        let key = make_smi(v);
        let hash = kata_rt_hash_int(key);
        s = kata_rt_set_insert(s, key, hash, eq, arena);
    }
    s
}

#[test]
fn set_basic_insert_contains() {
    let arena = kata_rt_arena_create();
    let eq = fn_ptr_as_i64(smi_eq);

    let s = build_set(arena, eq, &[1, 2, 3, 5, 8]);

    // Present keys
    for &v in &[1i64, 2, 3, 5, 8] {
        let key = make_smi(v);
        let hash = kata_rt_hash_int(key);
        assert_eq!(
            kata_rt_set_contains(s, key, hash, eq),
            1,
            "key {} should be contained",
            v
        );
    }

    // Absent keys
    for &v in &[4i64, 6, 7, 9, 10, 0, 100] {
        let key = make_smi(v);
        let hash = kata_rt_hash_int(key);
        assert_eq!(
            kata_rt_set_contains(s, key, hash, eq),
            0,
            "key {} should NOT be contained",
            v
        );
    }
}

#[test]
fn set_len() {
    let arena = kata_rt_arena_create();
    let eq = fn_ptr_as_i64(smi_eq);

    let s = build_set(arena, eq, &[1, 2, 3, 5, 8]);
    assert_eq!(kata_rt_set_len(s), 5);
}

#[test]
fn set_union() {
    let arena = kata_rt_arena_create();
    let eq = fn_ptr_as_i64(smi_eq);

    let a = build_set(arena, eq, &[1, 2, 3]);
    let b = build_set(arena, eq, &[3, 4, 5]);

    let u = kata_rt_set_union(a, b, eq, arena);
    assert_eq!(
        kata_rt_set_len(u),
        5,
        "union of {{1,2,3}} and {{3,4,5}} should have len=5"
    );

    // Verify all expected elements present
    for &v in &[1i64, 2, 3, 4, 5] {
        let key = make_smi(v);
        let hash = kata_rt_hash_int(key);
        assert_eq!(
            kata_rt_set_contains(u, key, hash, eq),
            1,
            "union should contain {}",
            v
        );
    }
}

#[test]
fn set_intersection() {
    let arena = kata_rt_arena_create();
    let eq = fn_ptr_as_i64(smi_eq);

    let a = build_set(arena, eq, &[1, 2, 3]);
    let b = build_set(arena, eq, &[3, 4, 5]);

    let i = kata_rt_set_intersection(a, b, eq, arena);
    assert_eq!(
        kata_rt_set_len(i),
        1,
        "intersection of {{1,2,3}} and {{3,4,5}} should have len=1"
    );

    // The only element should be 3
    let key = make_smi(3);
    let hash = kata_rt_hash_int(key);
    assert_eq!(
        kata_rt_set_contains(i, key, hash, eq),
        1,
        "intersection should contain 3"
    );
}

#[test]
fn set_difference() {
    let arena = kata_rt_arena_create();
    let eq = fn_ptr_as_i64(smi_eq);

    let a = build_set(arena, eq, &[1, 2, 3]);
    let b = build_set(arena, eq, &[3, 4, 5]);

    let d = kata_rt_set_difference(a, b, eq, arena);
    assert_eq!(
        kata_rt_set_len(d),
        2,
        "difference of {{1,2,3}} and {{3,4,5}} should have len=2"
    );

    // Should contain 1 and 2, but NOT 3
    for &v in &[1i64, 2] {
        let key = make_smi(v);
        let hash = kata_rt_hash_int(key);
        assert_eq!(
            kata_rt_set_contains(d, key, hash, eq),
            1,
            "difference should contain {}",
            v
        );
    }
    let key3 = make_smi(3);
    let hash3 = kata_rt_hash_int(key3);
    assert_eq!(
        kata_rt_set_contains(d, key3, hash3, eq),
        0,
        "difference should NOT contain 3"
    );
}

#[test]
fn set_persistence() {
    let arena = kata_rt_arena_create();
    let eq = fn_ptr_as_i64(smi_eq);

    let s = build_set(arena, eq, &[1, 2, 3]);
    assert_eq!(kata_rt_set_len(s), 3);

    let key = make_smi(42);
    let hash = kata_rt_hash_int(key);
    let s2 = kata_rt_set_insert(s, key, hash, eq, arena);

    // Original should be unchanged
    assert_eq!(kata_rt_set_len(s), 3, "original set should be unchanged");
    // New set should have 4 elements
    assert_eq!(kata_rt_set_len(s2), 4, "new set should have 4 elements");

    // Original should NOT contain 42
    assert_eq!(kata_rt_set_contains(s, key, hash, eq), 0);
    // New set should contain 42
    assert_eq!(kata_rt_set_contains(s2, key, hash, eq), 1);
}
