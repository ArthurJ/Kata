use super::*;

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

fn smi(n: i64) -> i64 {
    (n << 1) | 1
}

#[test]
fn test_hamt_two_int_inserts() {
    let _rt = TestRt::new();
    let arena = crate::arena::kata_rt_arena_create(_rt.rt_ptr);
    let dict = kata_rt_dict_empty(arena);
    let len0 = kata_rt_dict_len(dict);
    assert_eq!(
        len0,
        smi(0),
        "empty dict len should be SMI(0), got {}",
        len0
    );

    let key1 = 3i64;
    let val1 = 21i64;
    let hash1 = crate::hash::kata_rt_hash_int(key1);
    let eq_fn = crate::bigint::kata_rt_bi_eq as *const () as i64;
    let dict1 = kata_rt_dict_insert(dict, key1, val1, hash1, eq_fn, arena);
    let len1 = kata_rt_dict_len(dict1);
    assert_eq!(
        len1,
        smi(1),
        "after 1 insert, len should be SMI(1), got {}",
        len1
    );

    let key2 = 5i64;
    let val2 = 41i64;
    let hash2 = crate::hash::kata_rt_hash_int(key2);
    let dict2 = kata_rt_dict_insert(dict1, key2, val2, hash2, eq_fn, arena);
    let len2 = kata_rt_dict_len(dict2);
    assert_eq!(
        len2,
        smi(2),
        "after 2 inserts, len should be SMI(2), got {}",
        len2
    );

    let key3 = 7i64;
    let val3 = 61i64;
    let hash3 = crate::hash::kata_rt_hash_int(key3);
    let dict3 = kata_rt_dict_insert(dict2, key3, val3, hash3, eq_fn, arena);
    let len3 = kata_rt_dict_len(dict3);
    assert_eq!(
        len3,
        smi(3),
        "after 3 inserts, len should be SMI(3), got {}",
        len3
    );
}

#[test]
fn test_hamt_text_keys() {
    let _rt = TestRt::new();
    let arena = crate::arena::kata_rt_arena_create(_rt.rt_ptr);
    let dict = kata_rt_dict_empty(arena);

    let a = std::ffi::CString::new("a").unwrap();
    let b = std::ffi::CString::new("b").unwrap();
    let a_ptr = a.as_ptr() as i64;
    let b_ptr = b.as_ptr() as i64;

    let hash_a = crate::hash::kata_rt_hash_text(a_ptr);
    let hash_b = crate::hash::kata_rt_hash_text(b_ptr);

    let eq_fn = crate::text::kata_rt_string_eq as *const () as i64;

    let dict1 = kata_rt_dict_insert(dict, a_ptr, 3, hash_a, eq_fn, arena);
    let dict2 = kata_rt_dict_insert(dict1, b_ptr, 5, hash_b, eq_fn, arena);
    let len2 = kata_rt_dict_len(dict2);
    assert_eq!(
        len2,
        smi(2),
        "text-keyed dict with 2 entries should have len=SMI(2)"
    );
}
