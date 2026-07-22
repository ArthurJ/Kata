//! Set — persistent hash set built on top of Dict (HAMT).
//!
//! Set delegates to Dict with value = 0 (Unit). All persistence and
//! structural-sharing properties come from Dict for free.
//!
//! Set operations (union, intersection, difference) iterate the HAMT
//! via `collect_all_kvpairs` and delegate to Dict's insert/contains.

use crate::dict;

/// Allocate an empty set. Delegates to `kata_rt_dict_empty`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_empty(arena_handle: i64) -> i64 {
    dict::kata_rt_dict_empty(arena_handle)
}

/// Insert `elem` into the set. Returns a NEW set pointer (original unchanged).
/// Delegates to `kata_rt_dict_insert` with value = 0 (Unit).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_insert(
    set_ptr: i64,
    elem: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
    dict::kata_rt_dict_insert(set_ptr, elem, 0, hash, eq_fn, arena_handle)
}

/// Check if `elem` is in the set. Returns 1 (true) or 0 (false).
/// Delegates to `kata_rt_dict_contains`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_contains(set_ptr: i64, elem: i64, hash: i64, eq_fn: i64) -> i64 {
    dict::kata_rt_dict_contains(set_ptr, elem, hash, eq_fn)
}

/// Count elements in the set. Delegates to `kata_rt_dict_len`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_len(set_ptr: i64) -> i64 {
    dict::kata_rt_dict_len(set_ptr)
}

/// Remove `elem` from the set. Returns a NEW set pointer (original unchanged).
/// Delegates to `kata_rt_dict_remove` with value = 0.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_remove(
    set_ptr: i64,
    elem: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
    dict::kata_rt_dict_remove(set_ptr, elem, hash, eq_fn, arena_handle)
}

/// Iterate over the set. Returns `Optional::K` as a Sum box:
/// - tag=0 (Some): payload = key (the element)
/// - tag=1 (None): payload = 0
///
/// `iter_state` semantics (same as `kata_rt_dict_next`):
/// - 0: initialize — collect all KVPair pointers, return first key.
/// - N>0: return the Nth key (0-indexed).
/// - When N >= count: return None.
///
/// Unlike `kata_rt_dict_next` which returns a (K,V) tuple, this returns
/// just the key directly as the Sum payload.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_next(set_ptr: i64, iter_state: i64, arena_handle: i64) -> i64 {
    // Reuse the dict_next machinery — it populates the thread-local
    // iterator state and returns a Some(tuple) box. We extract the key
    // from the tuple instead of returning the tuple itself.
    //
    // But dict_next returns Some(payload=tuple_ptr) where tuple_ptr points
    // to a 16-byte (key, value) struct. We need just the key. We could:
    // 1. Call dict_next, read the key from the tuple, return Some(key).
    // 2. Or directly use collect_all_kvpairs and read keys.
    //
    // Approach 2 is cleaner — avoids allocating a tuple we don't need.
    // But we need to share the thread-local iter state with dict_next
    // (they use the same thread-locals). Since set_next and dict_next
    // are never interleaved (the caller uses one or the other), this is
    // fine.
    //
    // Actually, the simplest correct approach: call dict_next, extract
    // the key from the returned tuple, return a new Some(key) box.
    // This reuses all the iter_state logic perfectly.

    let result = dict::kata_rt_dict_next(set_ptr, iter_state, arena_handle);
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };

    if tag == 1 {
        // None — pass through.
        return result;
    }

    // Some — payload is a tuple pointer. Read key from offset 0.
    let tuple_ptr = unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
    let key = unsafe { std::ptr::read_unaligned(tuple_ptr as *const i64) };

    // Return Some(key) — tag=0, payload=key.
    crate::sum::kata_rt_store_sum_result(0, key, arena_handle)
}

// ── Set operations ─────────────────────────────────────────

/// Union of sets `a` and `b`. Returns a new set containing all elements
/// from both sets.
///
/// Strategy: start with `a` (structurally shared), iterate `b` and insert
/// each key into the result.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_union(a: i64, b: i64, eq_fn: i64, arena_handle: i64) -> i64 {
    // Start with a copy of a (insert into a builds a new tree sharing structure).
    // We need to iterate b and insert each key into a.
    let (arr, count) = unsafe { dict::collect_all_kvpairs(b, arena_handle) };

    let mut result = a;
    for i in 0..count {
        let kvptr = unsafe {
            std::ptr::read_unaligned((arr as *const u8).add((i as usize) * 8) as *const i64)
        };
        let key = unsafe { read_kvpair_key(kvptr) };
        let hash = unsafe { read_kvpair_hash(kvptr) };
        result = dict::kata_rt_dict_insert(result, key, 0, hash, eq_fn, arena_handle);
    }
    result
}

/// Intersection of sets `a` and `b`. Returns a new set containing elements
/// that are in BOTH `a` and `b`.
///
/// Strategy: start with empty set, iterate `a`, for each key check if it's
/// in `b` (dict_contains). If yes, insert into result.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_intersection(a: i64, b: i64, eq_fn: i64, arena_handle: i64) -> i64 {
    let (arr, count) = unsafe { dict::collect_all_kvpairs(a, arena_handle) };

    let mut result = dict::kata_rt_dict_empty(arena_handle);
    for i in 0..count {
        let kvptr = unsafe {
            std::ptr::read_unaligned((arr as *const u8).add((i as usize) * 8) as *const i64)
        };
        let key = unsafe { read_kvpair_key(kvptr) };
        let hash = unsafe { read_kvpair_hash(kvptr) };

        if dict::kata_rt_dict_contains(b, key, hash, eq_fn) == 1 {
            result = dict::kata_rt_dict_insert(result, key, 0, hash, eq_fn, arena_handle);
        }
    }
    result
}

/// Difference of sets `a` and `b` (a \ b). Returns a new set containing
/// elements in `a` that are NOT in `b`.
///
/// Strategy: start with empty set, iterate `a`, for each key check if it's
/// NOT in `b`. If not in b, insert into result.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_difference(a: i64, b: i64, eq_fn: i64, arena_handle: i64) -> i64 {
    let (arr, count) = unsafe { dict::collect_all_kvpairs(a, arena_handle) };

    let mut result = dict::kata_rt_dict_empty(arena_handle);
    for i in 0..count {
        let kvptr = unsafe {
            std::ptr::read_unaligned((arr as *const u8).add((i as usize) * 8) as *const i64)
        };
        let key = unsafe { read_kvpair_key(kvptr) };
        let hash = unsafe { read_kvpair_hash(kvptr) };

        if dict::kata_rt_dict_contains(b, key, hash, eq_fn) == 0 {
            result = dict::kata_rt_dict_insert(result, key, 0, hash, eq_fn, arena_handle);
        }
    }
    result
}

// ── Helpers (same layout as dict.rs KVPair) ─────────────────

/// Read the key from a KVPair (offset 0).
unsafe fn read_kvpair_key(leaf_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned(leaf_ptr as *const i64) }
}

/// Read the hash from a KVPair (offset 16).
unsafe fn read_kvpair_hash(leaf_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned((leaf_ptr as *const u8).add(16) as *const i64) }
}