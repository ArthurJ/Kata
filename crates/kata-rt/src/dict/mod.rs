//! Dict — Hash Array Mapped Trie (HAMT) persistent dictionary
//! with Cons-list overlay for insertion-order iteration.
//!
//! ## Public Dict layout (16 bytes in arena)
//! ```text
//! offset 0: hamt_root (i64) — HAMT for O(log n) lookup
//! offset 8: insert_log (i64) — Cons list of KVPair pointers for iteration
//! ```
//!
//! The HAMT provides O(log n) lookup/insert/remove with structural sharing.
//! The Cons list records insertion order (newest first via prepend).
//! During iteration we walk the Cons list, dedup by key, and skip removed
//! entries via `hamt_contains`.
//!
//! ## HAMT node layouts
//!
//! **Interior node** (variable size):
//! ```text
//! offset 0:    bitmap (u32 stored as i64, padded to 8 bytes)
//! offset 8+:   dense children array (popcount(bitmap) entries, each i64)
//! ```
//!
//! **KVPair** (leaf entry) — 24 bytes:
//! ```text
//! offset 0:  key (i64)
//! offset 8:  value (i64)
//! offset 16: hash (i64) — stored to avoid recomputation during leaf splitting
//! ```
//!
//! **Collision node** (hashes fully collide but keys differ):
//! ```text
//! offset 0:    sentinel tag = -1 (i64)
//! offset 8:    count (i64)
//! offset 16+:  array of KVPair pointers (count entries)
//! ```
//!
//! ## Pointer tagging for children
//!
//! Arena allocations are 8-byte aligned (bits 0-2 = 0). We use bits 0 and 1
//! to distinguish the three node types in the children array:
//! - bit 0 = 1: tagged leaf pointer (KVPair). Untag with `ptr & !1`.
//! - bit 1 = 1 (bit 0 = 0): tagged collision node pointer. Untag with `ptr & !2`.
//! - bits 0,1 = 0: interior node pointer (untagged).

mod hamt;

pub(crate) use hamt::collect_all_kvpairs;

use hamt::{
    hamt_contains, hamt_empty, hamt_find_kvpair, hamt_get_checked, hamt_insert, hamt_len,
    hamt_remove, make_kv_tuple, read_kvpair_hash, read_kvpair_key, read_kvpair_value,
};

const HASH_BITS: u32 = 5;
const HASH_MASK: u64 = 0x1f;
const HASH_LEVELS: u32 = 6;

/// Bit 0 set → leaf (KVPair) pointer.
const LEAF_TAG: i64 = 1;
/// Bit 1 set → collision node pointer.
const COLLISION_TAG: i64 = 2;
/// Sentinel stored at offset 0 inside a collision node.
const COLLISION_SENTINEL: i64 = -1;

/// Equality function pointer type.
pub(super) type EqFn = extern "C" fn(i64, i64) -> i64;

// ── Thread-local iterator state (HAMT-order, used by hamt_next) ──

use std::cell::Cell;

thread_local! {
    pub(super) static ITER_ARRAY: Cell<i64> = const { Cell::new(0) };
    pub(super) static ITER_COUNT: Cell<i64> = const { Cell::new(0) };
}

// ════════════════════════════════════════════════════════════════
// Public Dict API — 16-byte struct wrappers
// ════════════════════════════════════════════════════════════════

/// Dict is a 16-byte struct: (hamt_root, insert_log)
/// offset 0: hamt_root, offset 8: insert_log (Cons list of KVPair ptrs)
///
/// Allocate an empty Dict (16-byte struct: hamt_root + nil insert_log).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_dict_empty(arena_handle: i64) -> i64 {
    let hamt = hamt_empty(arena_handle);
    // Allocate 16-byte Dict struct
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 16);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, hamt); // hamt_root
        std::ptr::write_unaligned((ptr as *mut u8).add(8) as *mut i64, 0); // insert_log = nil
    }
    ptr
}

/// Insert (key, value) into the dict. Returns a NEW Dict pointer.
/// The original dict is unchanged (structural sharing).
///
/// `hash` is pre-computed by the caller (via HASHABLE dispatch).
/// `eq_fn` is a function pointer: `extern "C" fn(i64, i64) -> i64` (1 = equal, 0 = not).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_dict_insert(
    dict_ptr: i64,
    key: i64,
    value: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
    let hamt_root = unsafe { std::ptr::read_unaligned(dict_ptr as *const i64) };
    let old_log = unsafe { std::ptr::read_unaligned((dict_ptr as *const u8).add(8) as *const i64) };

    // 1. HAMT insert — returns (new_root, kvpair_ptr)
    let (new_hamt, kvpair_ptr) = hamt_insert(hamt_root, key, value, hash, eq_fn, arena_handle);

    // 2. Cons prepend: new_log = Cons(kvpair_ptr, old_log)
    let cons_cell = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 16);
    if cons_cell == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(cons_cell as *mut i64, kvpair_ptr); // head
        std::ptr::write_unaligned((cons_cell as *mut u8).add(8) as *mut i64, old_log); // tail
    }
    let new_log = cons_cell;

    // 3. Allocate new Dict struct (16 bytes) with (new_hamt, new_log)
    let new_dict = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 16);
    if new_dict == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(new_dict as *mut i64, new_hamt);
        std::ptr::write_unaligned((new_dict as *mut u8).add(8) as *mut i64, new_log);
    }
    new_dict
}

/// Get the value for `key`. Returns a Result box (Sum):
/// - Ok:  tag=0, payload=value
/// - Err: tag=1, payload=0
///
/// `arena_handle` is needed to allocate the Result box.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_dict_get_checked(
    dict_ptr: i64,
    key: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
    let hamt_root = unsafe { std::ptr::read_unaligned(dict_ptr as *const i64) };
    hamt_get_checked(hamt_root, key, hash, eq_fn, arena_handle)
}

/// Check if `key` is present in the dict. Returns 1 (true) or 0 (false).
/// Does NOT allocate a Result box — just returns 1 or 0 directly.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_dict_contains(dict_ptr: i64, key: i64, hash: i64, eq_fn: i64) -> i64 {
    let hamt_root = unsafe { std::ptr::read_unaligned(dict_ptr as *const i64) };
    hamt_contains(hamt_root, key, hash, eq_fn)
}

/// Count entries by traversing the HAMT. O(n).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_dict_len(dict_ptr: i64) -> i64 {
    let hamt_root = unsafe { std::ptr::read_unaligned(dict_ptr as *const i64) };
    let count = hamt_len(hamt_root);
    // SMI tag: (val << 1) | 1
    (count << 1) | 1
}

/// Remove `key` from the dict. Returns a NEW Dict pointer (original unchanged).
///
/// The Cons list is NOT modified on remove — the old KVPair stays in the
/// log but is skipped during iteration (checked via hamt_contains).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_dict_remove(
    dict_ptr: i64,
    key: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
    let hamt_root = unsafe { std::ptr::read_unaligned(dict_ptr as *const i64) };
    let old_log = unsafe { std::ptr::read_unaligned((dict_ptr as *const u8).add(8) as *const i64) };

    let new_hamt = hamt_remove(hamt_root, key, hash, eq_fn, arena_handle);

    // Allocate new Dict struct with (new_hamt, old_log) — Cons list unchanged
    let new_dict = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 16);
    if new_dict == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(new_dict as *mut i64, new_hamt);
        std::ptr::write_unaligned((new_dict as *mut u8).add(8) as *mut i64, old_log);
    }
    new_dict
}

/// Iterate over the dict in insertion order (newest first).
/// Returns `Optional::(K, V)` as a Sum box:
/// - tag=0 (Some): payload = pointer to a 16-byte tuple (key at 0, value at 8)
/// - tag=1 (None): payload = 0
///
/// `iter_state` semantics:
/// - 0: initialize — walk the Cons list, dedup by key, collect valid KVPair
///   pointers into an arena-allocated array (stored in thread-local), return
///   the first entry (or None if empty).
/// - N>0: return the Nth entry (0-indexed) from the pre-collected array.
/// - When N >= count: return None (tag=1).
///
/// Dedup ensures that if a key was replaced (appears multiple times in the
/// Cons list), only the newest occurrence is kept. Removed keys (not in HAMT)
/// are skipped via `hamt_contains`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_dict_next(dict_ptr: i64, iter_state: i64, arena_handle: i64) -> i64 {
    if iter_state == 0 {
        // Initialize: walk the Cons list, dedup, collect valid KVPair pointers.
        let hamt_root = unsafe { std::ptr::read_unaligned(dict_ptr as *const i64) };
        let insert_log =
            unsafe { std::ptr::read_unaligned((dict_ptr as *const u8).add(8) as *const i64) };

        let eq_fn_for_contains = 0i64; // We use key comparison via hamt_contains

        // Walk the Cons list (newest → oldest), dedup by key, check hamt_contains.
        let mut kvpair_ptrs: Vec<i64> = Vec::new();
        let mut seen_keys: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut current = insert_log;
        while current != 0 {
            let kvpair_ptr = unsafe { std::ptr::read_unaligned(current as *const i64) };
            let tail =
                unsafe { std::ptr::read_unaligned((current as *const u8).add(8) as *const i64) };

            if kvpair_ptr != 0 {
                let key = unsafe { read_kvpair_key(kvpair_ptr) };
                let hash = unsafe { read_kvpair_hash(kvpair_ptr) };

                // Dedup: skip if we've already seen this key (newer entry wins).
                if !seen_keys.contains(&key) {
                    // Check if this key is still present in the HAMT (not removed).
                    // We need eq_fn for hamt_contains, but we don't have it here.
                    // However, hamt_contains needs eq_fn as i64. We don't have it.
                    //
                    // Alternative: since we have the hash, and the HAMT stores
                    // the same KVPair pointers, we can check if the KVPair pointer
                    // in the Cons list matches the one in the HAMT. But that
                    // requires traversing the HAMT to find the leaf.
                    //
                    // Simplest approach: just include all entries and let the
                    // caller handle dedup. But the PRD says to skip removed entries.
                    //
                    // For now, we include all deduped entries. The remove case
                    // is handled by the caller checking if the key is still present.
                    // Actually, per the PRD: "For each KVPair pointer, check
                    // hamt_contains(hamt_root, key, hash, eq_fn). If present and
                    // not seen before, include it."
                    //
                    // We don't have eq_fn here. We need to pass it through.
                    // The function signature doesn't include eq_fn.
                    //
                    // WORKAROUND: Instead of hamt_contains, we can do a simpler
                    // check: walk the HAMT to find the leaf at this hash and
                    // compare the KVPair pointer. If they match, the entry is
                    // current. If not (different pointer), the entry was replaced
                    // or removed.
                    //
                    // Actually, the simplest correct approach: we store the eq_fn
                    // in a thread-local during the first call. But that requires
                    // the caller to pass it.
                    //
                    // PRAGMATIC: Since dict_next doesn't receive eq_fn, and the
                    // PRD says to use hamt_contains, we need another approach.
                    // We'll just include all deduped entries (by key). Removed
                    // keys will still appear in iteration — but wait, the HAMT
                    // no longer has them, so the KVPair pointer in the Cons list
                    // points to an orphaned KVPair. The value is still there
                    // (arena-allocated, not freed), so the tuple will still
                    // contain the old value. This is incorrect for removed keys.
                    //
                    // BEST APPROACH without eq_fn: walk the HAMT to find the
                    // leaf at the given hash, compare the pointer. This works
                    // without eq_fn because we're comparing pointers, not keys.
                    let in_hamt = unsafe { hamt_find_kvpair(hamt_root, hash, kvpair_ptr, 0) };

                    if in_hamt {
                        seen_keys.insert(key);
                        kvpair_ptrs.push(kvpair_ptr);
                    }
                }
            }

            current = tail;
        }

        let _ = eq_fn_for_contains; // suppress unused warning

        let count = kvpair_ptrs.len() as i64;
        if count == 0 {
            ITER_ARRAY.with(|c| c.set(0));
            ITER_COUNT.with(|c| c.set(0));
            return crate::sum::kata_rt_store_sum_result(1, 0, arena_handle);
        }

        // Allocate array in arena: count * 8 bytes.
        let arr_size = count * 8;
        let arr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, arr_size);
        if arr == 0 {
            return crate::sum::kata_rt_store_sum_result(1, 0, arena_handle);
        }
        for (i, &kvptr) in kvpair_ptrs.iter().enumerate() {
            unsafe {
                std::ptr::write_unaligned((arr as *mut u8).add(i * 8) as *mut i64, kvptr);
            }
        }

        ITER_ARRAY.with(|c| c.set(arr));
        ITER_COUNT.with(|c| c.set(count));

        // Return first entry (index 0).
        return unsafe { make_kv_tuple(arr, 0, arena_handle) };
    }

    // iter_state > 0 — index into array.
    let arr = ITER_ARRAY.with(|c| c.get());
    let count = ITER_COUNT.with(|c| c.get());

    if arr == 0 || iter_state >= count {
        // Exhausted — return None.
        return crate::sum::kata_rt_store_sum_result(1, 0, arena_handle);
    }

    unsafe { make_kv_tuple(arr, iter_state, arena_handle) }
}

/// Merge two dicts (right-biased union).
///
/// `kata_rt_dict_merge(a, b, eq_fn, arena) -> i64`
///
/// Iterates `b` and inserts each (key, value) into `a`. When a key exists
/// in both, the value from `b` overwrites the value from `a` (right-biased).
/// Returns a new Dict pointer.
///
/// `eq_fn` is the function pointer for key equality comparison.
/// Hash is read from the KVpair (stored at insert time).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_dict_merge(a: i64, b: i64, eq_fn: i64, arena_handle: i64) -> i64 {
    // Collect all KVPair pointers from b.
    let b_hamt = unsafe { std::ptr::read_unaligned(b as *const i64) };
    let (arr, count) = unsafe { collect_all_kvpairs(b_hamt, arena_handle) };

    let mut result = a;
    for i in 0..count {
        let kvptr = unsafe {
            std::ptr::read_unaligned((arr as *const u8).add((i as usize) * 8) as *const i64)
        };
        let key = unsafe { read_kvpair_key(kvptr) };
        let val = unsafe { read_kvpair_value(kvptr) };
        let hash = unsafe { read_kvpair_hash(kvptr) };
        result = kata_rt_dict_insert(result, key, val, hash, eq_fn, arena_handle);
    }
    result
}

#[cfg(test)]
mod tests {
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
}
