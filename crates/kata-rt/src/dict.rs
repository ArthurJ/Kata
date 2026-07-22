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
type EqFn = extern "C" fn(i64, i64) -> i64;

// ── Helpers ──────────────────────────────────────────────

/// Extract the 5-bit index at the given depth from a hash.
#[inline]
fn hash_index(hash: i64, depth: u32) -> usize {
    ((hash as u64 >> (depth * HASH_BITS)) & HASH_MASK) as usize
}

/// Population count of a u32 bitmap.
#[inline]
fn popcount(bitmap: u32) -> usize {
    bitmap.count_ones() as usize
}

/// Compacted index into the dense children array for logical index `idx`.
/// This is the popcount of bits below `idx` in the bitmap.
#[inline]
fn child_index(bitmap: u32, idx: usize) -> usize {
    if idx == 0 {
        return 0;
    }
    let mask = (1u32 << idx) - 1;
    popcount(bitmap & mask)
}

/// Check if a child pointer is a tagged leaf (KVPair).
#[inline]
fn is_leaf(ptr: i64) -> bool {
    (ptr & LEAF_TAG) == LEAF_TAG
}

/// Check if a child pointer is a tagged collision node.
#[inline]
fn is_collision(ptr: i64) -> bool {
    (ptr & COLLISION_TAG) == COLLISION_TAG
}

/// Remove the leaf tag from a tagged leaf pointer.
#[inline]
fn untag_leaf(ptr: i64) -> i64 {
    ptr & !LEAF_TAG
}

/// Add the leaf tag to a KVPair pointer.
#[inline]
fn tag_leaf(ptr: i64) -> i64 {
    ptr | LEAF_TAG
}

/// Remove the collision tag from a tagged collision pointer.
#[inline]
fn untag_collision(ptr: i64) -> i64 {
    ptr & !COLLISION_TAG
}

/// Add the collision tag to a collision node pointer.
#[inline]
fn tag_collision(ptr: i64) -> i64 {
    ptr | COLLISION_TAG
}

/// Allocate a KVPair (24 bytes: key, value, hash).
fn alloc_kvpair(key: i64, value: i64, hash: i64, arena: i64) -> i64 {
    let ptr = crate::arena::kata_rt_arena_alloc(arena, 24);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, key);
        std::ptr::write_unaligned((ptr as *mut u8).add(8) as *mut i64, value);
        std::ptr::write_unaligned((ptr as *mut u8).add(16) as *mut i64, hash);
    }
    ptr
}

/// Read the key from a KVPair.
unsafe fn read_kvpair_key(leaf_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned(leaf_ptr as *const i64) }
}

/// Read the value from a KVPair.
unsafe fn read_kvpair_value(leaf_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned((leaf_ptr as *const u8).add(8) as *const i64) }
}

/// Read the hash from a KVPair.
unsafe fn read_kvpair_hash(leaf_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned((leaf_ptr as *const u8).add(16) as *const i64) }
}

/// Allocate an interior node with the given bitmap and children.
fn alloc_node(bitmap: u32, children: &[i64], arena: i64) -> i64 {
    let header_size = 8; // bitmap as i64
    let children_size = children.len() * 8;
    let total = header_size + children_size;
    let ptr = crate::arena::kata_rt_arena_alloc(arena, total as i64);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, bitmap as i64);
        for (i, &child) in children.iter().enumerate() {
            std::ptr::write_unaligned((ptr as *mut u8).add(header_size + i * 8) as *mut i64, child);
        }
    }
    ptr
}

/// Allocate a collision node with the given entries (KVPair pointers).
fn alloc_collision(count: i64, entries: &[i64], arena: i64) -> i64 {
    let total = 16 + entries.len() * 8;
    let ptr = crate::arena::kata_rt_arena_alloc(arena, total as i64);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, COLLISION_SENTINEL);
        std::ptr::write_unaligned((ptr as *mut u8).add(8) as *mut i64, count);
        for (i, &entry) in entries.iter().enumerate() {
            std::ptr::write_unaligned((ptr as *mut u8).add(16 + i * 8) as *mut i64, entry);
        }
    }
    ptr
}

/// Read the bitmap from an interior node.
unsafe fn read_bitmap(node_ptr: i64) -> u32 {
    unsafe { std::ptr::read_unaligned(node_ptr as *const i64) as u32 }
}

/// Read a child at the given compacted index from an interior node.
unsafe fn read_child(node_ptr: i64, compacted_idx: usize) -> i64 {
    unsafe {
        std::ptr::read_unaligned((node_ptr as *const u8).add(8 + compacted_idx * 8) as *const i64)
    }
}

/// Read all children from an interior node.
unsafe fn read_all_children(node_ptr: i64, count: usize) -> Vec<i64> {
    (0..count)
        .map(|i| unsafe { read_child(node_ptr, i) })
        .collect()
}

/// Read the count from a collision node.
unsafe fn read_collision_count(node_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned((node_ptr as *const u8).add(8) as *const i64) }
}

/// Read a KVPair pointer from a collision node at index `i`.
unsafe fn read_collision_entry(node_ptr: i64, i: i64) -> i64 {
    unsafe {
        std::ptr::read_unaligned((node_ptr as *const u8).add(16 + (i as usize) * 8) as *const i64)
    }
}

// ════════════════════════════════════════════════════════════════
// Internal HAMT functions (pub(crate)) — operate on raw HAMT root
// ════════════════════════════════════════════════════════════════

/// Allocate an empty HAMT root node (bitmap = 0, no children).
///
/// Returns a pointer to the root node.
pub(crate) fn hamt_empty(arena_handle: i64) -> i64 {
    // Root node: 8 bytes (bitmap=0 as i64). No children.
    let ptr = crate::arena::kata_rt_arena_alloc(arena_handle, 8);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, 0);
    }
    ptr
}

/// Insert (key, value) into the HAMT. Returns (new_root, kvpair_ptr).
///
/// The `kvpair_ptr` is the UNTAGGED pointer to the KVPair that was
/// inserted or replaced. Used by the public `kata_rt_dict_insert` to
/// prepend to the Cons list.
pub(crate) fn hamt_insert(
    hamt_root: i64,
    key: i64,
    value: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> (i64, i64) {
    let eq = unsafe { std::mem::transmute::<i64, EqFn>(eq_fn) };
    unsafe { insert_recursive(hamt_root, key, value, hash, 0, eq, arena_handle) }
}

/// Get the value for `key`. Returns a Result box (Sum):
/// - Ok:  tag=0, payload=value
/// - Err: tag=1, payload=0
///
/// `arena_handle` is needed to allocate the Result box.
pub(crate) fn hamt_get_checked(
    hamt_root: i64,
    key: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
    let eq = unsafe { std::mem::transmute::<i64, EqFn>(eq_fn) };
    unsafe { get_recursive(hamt_root, key, hash, 0, eq, arena_handle) }
}

/// Check if `key` is present in the HAMT. Returns 1 (true) or 0 (false).
/// Does NOT allocate a Result box — just returns 1 or 0 directly.
pub(crate) fn hamt_contains(hamt_root: i64, key: i64, hash: i64, eq_fn: i64) -> i64 {
    let eq = unsafe { std::mem::transmute::<i64, EqFn>(eq_fn) };
    unsafe { contains_recursive(hamt_root, key, hash, 0, eq) }
}

/// Count entries by traversing the trie. O(n).
pub(crate) fn hamt_len(hamt_root: i64) -> i64 {
    unsafe { count_recursive(hamt_root) }
}

/// Remove `key` from the HAMT. Returns a NEW root pointer (original unchanged).
pub(crate) fn hamt_remove(
    hamt_root: i64,
    key: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
    let eq = unsafe { std::mem::transmute::<i64, EqFn>(eq_fn) };
    unsafe { remove_recursive(hamt_root, key, hash, 0, eq, arena_handle) }
}

/// Iterate over the HAMT via `collect_all_kvpairs`. Returns `Optional::(K, V)`
/// as a Sum box. Used by Set (which needs HAMT-order iteration).
#[allow(dead_code)]
pub(crate) fn hamt_next(hamt_root: i64, iter_state: i64, arena_handle: i64) -> i64 {
    if iter_state == 0 {
        let (arr, count) = unsafe { collect_all_kvpairs(hamt_root, arena_handle) };
        ITER_ARRAY.with(|c| c.set(arr));
        ITER_COUNT.with(|c| c.set(count));

        if count == 0 {
            return crate::sum::kata_rt_store_sum_result(1, 0, arena_handle);
        }
        return unsafe { make_kv_tuple(arr, 0, arena_handle) };
    }

    let arr = ITER_ARRAY.with(|c| c.get());
    let count = ITER_COUNT.with(|c| c.get());

    if arr == 0 || iter_state >= count {
        return crate::sum::kata_rt_store_sum_result(1, 0, arena_handle);
    }

    unsafe { make_kv_tuple(arr, iter_state, arena_handle) }
}

/// Collect all KVPair pointers from the HAMT into a flat arena-allocated array.
///
/// Returns (array_ptr, count). The array contains `count` KVPair pointers
/// (untagged), each pointing to a 24-byte KVPair struct.
///
/// This is the internal helper used by both `hamt_next` and the set
/// operations (union/intersection/difference) in `set.rs`.
pub(crate) unsafe fn collect_all_kvpairs(hamt_root: i64, arena_handle: i64) -> (i64, i64) {
    // First pass: collect into a Vec.
    let mut kvpair_ptrs: Vec<i64> = Vec::new();
    unsafe { collect_recursive(hamt_root, &mut kvpair_ptrs) };

    let count = kvpair_ptrs.len() as i64;
    if count == 0 {
        // Return a dummy empty array (1 entry = 0, count = 0).
        let arr = crate::arena::kata_rt_arena_alloc(arena_handle, 8);
        if arr != 0 {
            unsafe { std::ptr::write_unaligned(arr as *mut i64, 0) };
        }
        return (arr, 0);
    }

    // Allocate array in arena: count * 8 bytes.
    let arr_size = count * 8;
    let arr = crate::arena::kata_rt_arena_alloc(arena_handle, arr_size);
    if arr == 0 {
        return (0, 0);
    }
    for (i, &kvptr) in kvpair_ptrs.iter().enumerate() {
        unsafe {
            std::ptr::write_unaligned((arr as *mut u8).add(i * 8) as *mut i64, kvptr);
        }
    }
    (arr, count)
}

// ── Recursive implementations ─────────────────────────────

/// Recursive insert with copy-on-write.
///
/// Returns `(new_node_ptr, kvpair_ptr)` where `kvpair_ptr` is the
/// UNTAGGED pointer to the KVPair that was inserted or replaced.
///
/// At each level:
/// 1. Extract 5-bit index from hash at current depth.
/// 2. Check bitmap: is this index present?
/// 3. If no: set bit, allocate new node with this entry added.
/// 4. If yes: recurse into child:
///    - If child is leaf (KVPair): compare keys
///      - Same key: replace value (allocate new KVPair)
///      - Different key, same remaining hash bits: create collision node
///      - Different key, different hash bits: create interior node, recurse both
///    - If child is collision: extend it
///    - If child is interior: recurse
/// 5. Copy the current node with updated child pointer.
/// 6. Return (new root, kvpair_ptr).
unsafe fn insert_recursive(
    node_ptr: i64,
    key: i64,
    value: i64,
    hash: i64,
    depth: u32,
    eq_fn: EqFn,
    arena: i64,
) -> (i64, i64) {
    if depth >= HASH_LEVELS {
        // Collision level — create/extend collision node.
        // At this point, `node_ptr` points to an existing child (leaf or collision).
        return unsafe { insert_at_collision_level(node_ptr, key, value, hash, eq_fn, arena) };
    }

    let bitmap = unsafe { read_bitmap(node_ptr) };
    let idx = hash_index(hash, depth);
    let bit = 1u32 << idx;

    if bitmap & bit == 0 {
        // No child at this index — insert new leaf.
        let leaf = alloc_kvpair(key, value, hash, arena);
        let tagged_leaf = tag_leaf(leaf);
        let new_bitmap = bitmap | bit;
        let pos = child_index(new_bitmap, idx);
        let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
        let mut new_children = children;
        new_children.insert(pos, tagged_leaf);
        let new_node = alloc_node(new_bitmap, &new_children, arena);
        (new_node, leaf)
    } else {
        // Child exists at this index — recurse or split.
        let pos = child_index(bitmap, idx);
        let child = unsafe { read_child(node_ptr, pos) };

        if is_collision(child) {
            // Collision node — extend it.
            let coll_ptr = untag_collision(child);
            let (new_coll, kvpair) =
                unsafe { extend_collision(coll_ptr, key, value, hash, eq_fn, arena) };
            let new_child = tag_collision(new_coll);
            let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
            let mut new_children = children;
            new_children[pos] = new_child;
            let new_node = alloc_node(bitmap, &new_children, arena);
            (new_node, kvpair)
        } else if is_leaf(child) {
            // Existing leaf — compare keys.
            let leaf_ptr = untag_leaf(child);
            let existing_key = unsafe { read_kvpair_key(leaf_ptr) };

            if eq_fn(existing_key, key) == 1 {
                // Same key — replace value.
                let new_leaf = alloc_kvpair(key, value, hash, arena);
                let tagged = tag_leaf(new_leaf);
                let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
                let mut new_children = children;
                new_children[pos] = tagged;
                let new_node = alloc_node(bitmap, &new_children, arena);
                (new_node, new_leaf)
            } else {
                // Different key — need to split.
                let existing_hash = unsafe { read_kvpair_hash(leaf_ptr) };
                if depth + 1 >= HASH_LEVELS {
                    // We're at the last level — both hashes are the same
                    // (they collided all the way down). Create a collision node.
                    let new_leaf = alloc_kvpair(key, value, hash, arena);
                    let coll = alloc_collision(2, &[leaf_ptr, new_leaf], arena);
                    let tagged_coll = tag_collision(coll);
                    let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
                    let mut new_children = children;
                    new_children[pos] = tagged_coll;
                    let new_node = alloc_node(bitmap, &new_children, arena);
                    (new_node, new_leaf)
                } else {
                    // Hashes may diverge at a deeper level.
                    // Create a new interior node and insert both leaves into it.
                    let (new_interior, kvpair) = unsafe {
                        split_leaves(
                            leaf_ptr,
                            existing_hash,
                            key,
                            value,
                            hash,
                            depth + 1,
                            eq_fn,
                            arena,
                        )
                    };
                    let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
                    let mut new_children = children;
                    new_children[pos] = new_interior;
                    let new_node = alloc_node(bitmap, &new_children, arena);
                    (new_node, kvpair)
                }
            }
        } else {
            // Interior node — recurse.
            let (new_child, kvpair) =
                unsafe { insert_recursive(child, key, value, hash, depth + 1, eq_fn, arena) };
            let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
            let mut new_children = children;
            new_children[pos] = new_child;
            let new_node = alloc_node(bitmap, &new_children, arena);
            (new_node, kvpair)
        }
    }
}

/// Split two leaves into a new interior node subtree.
///
/// Returns `(new_interior_ptr, kvpair_ptr)`.
///
/// `existing_leaf_ptr` — pointer to the existing KVPair (untagged).
/// `existing_hash` — the existing leaf's hash.
/// `new_key`, `new_value`, `new_hash` — the new entry.
/// `depth` — the depth at which the new interior node starts.
///
/// Creates a new interior node containing both leaves, potentially
/// recursing deeper if they share the same 5-bit index at this level.
#[allow(clippy::too_many_arguments)]
unsafe fn split_leaves(
    existing_leaf_ptr: i64,
    existing_hash: i64,
    new_key: i64,
    new_value: i64,
    new_hash: i64,
    depth: u32,
    eq_fn: EqFn,
    arena: i64,
) -> (i64, i64) {
    // Allocate new leaf for the new entry.
    let new_leaf = alloc_kvpair(new_key, new_value, new_hash, arena);

    // Reuse existing leaf pointer (re-tag it).
    let existing_tagged = tag_leaf(existing_leaf_ptr);

    if depth >= HASH_LEVELS {
        // Both hashes exhausted — create collision node.
        let coll = alloc_collision(2, &[existing_leaf_ptr, new_leaf], arena);
        return (tag_collision(coll), new_leaf);
    }

    let existing_idx = hash_index(existing_hash, depth);
    let new_idx = hash_index(new_hash, depth);

    if existing_idx == new_idx {
        // Same index at this level — recurse deeper.
        // Create a temporary single-child node for the existing leaf,
        // then insert the new leaf into it.
        let temp_node = alloc_node(1u32 << existing_idx, &[existing_tagged], arena);
        unsafe { insert_recursive(temp_node, new_key, new_value, new_hash, depth, eq_fn, arena) }
    } else {
        // Different indices — create a node with both.
        let mut bitmap = 0u32;
        bitmap |= 1u32 << existing_idx;
        bitmap |= 1u32 << new_idx;

        let mut children = vec![0i64; 2];
        // Place in the correct compacted positions.
        let existing_pos = child_index(bitmap, existing_idx);
        let new_pos = child_index(bitmap, new_idx);
        children[existing_pos] = existing_tagged;
        children[new_pos] = tag_leaf(new_leaf);

        let node = alloc_node(bitmap, &children, arena);
        (node, new_leaf)
    }
}

/// Insert into a collision-level node. `node_ptr` may be a leaf (tagged or
/// untagged) or a collision node (tagged).
///
/// Returns `(new_node_ptr_tagged, kvpair_ptr)`.
unsafe fn insert_at_collision_level(
    node_ptr: i64,
    key: i64,
    value: i64,
    hash: i64,
    eq_fn: EqFn,
    arena: i64,
) -> (i64, i64) {
    // node_ptr could be a tagged leaf, tagged collision, or untagged leaf.
    if is_collision(node_ptr) {
        let coll_ptr = untag_collision(node_ptr);
        let (new_coll, kvpair) =
            unsafe { extend_collision(coll_ptr, key, value, hash, eq_fn, arena) };
        return (tag_collision(new_coll), kvpair);
    }

    // It's a leaf (could be tagged or untagged).
    let leaf_ptr = if is_leaf(node_ptr) {
        untag_leaf(node_ptr)
    } else {
        node_ptr
    };
    let existing_key = unsafe { read_kvpair_key(leaf_ptr) };

    if eq_fn(existing_key, key) == 1 {
        // Same key — replace.
        let new_leaf = alloc_kvpair(key, value, hash, arena);
        // Return as untagged leaf (caller at collision level handles it).
        return (new_leaf, new_leaf);
    }

    // Different key, same hash — create collision node.
    let new_leaf = alloc_kvpair(key, value, hash, arena);
    let coll = alloc_collision(2, &[leaf_ptr, new_leaf], arena);
    (tag_collision(coll), new_leaf)
}

/// Extend an existing collision node with a new entry, or replace
/// if the key already exists. Returns (untagged collision node ptr, kvpair_ptr).
unsafe fn extend_collision(
    coll_ptr: i64,
    key: i64,
    value: i64,
    hash: i64,
    eq_fn: EqFn,
    arena: i64,
) -> (i64, i64) {
    let count = unsafe { read_collision_count(coll_ptr) };

    // Check if key already exists.
    for i in 0..count {
        let entry_ptr = unsafe { read_collision_entry(coll_ptr, i) };
        let existing_key = unsafe { read_kvpair_key(entry_ptr) };
        if eq_fn(existing_key, key) == 1 {
            // Replace this entry.
            let new_entry = alloc_kvpair(key, value, hash, arena);
            let mut entries = Vec::with_capacity(count as usize);
            for j in 0..count {
                if j == i {
                    entries.push(new_entry);
                } else {
                    entries.push(unsafe { read_collision_entry(coll_ptr, j) });
                }
            }
            return (alloc_collision(count, &entries, arena), new_entry);
        }
    }

    // New key — append.
    let new_entry = alloc_kvpair(key, value, hash, arena);
    let mut entries = Vec::with_capacity((count + 1) as usize);
    for j in 0..count {
        entries.push(unsafe { read_collision_entry(coll_ptr, j) });
    }
    entries.push(new_entry);
    (alloc_collision(count + 1, &entries, arena), new_entry)
}

/// Recursive get. Returns Result box (Ok=value, Err=0).
unsafe fn get_recursive(
    node_ptr: i64,
    key: i64,
    hash: i64,
    depth: u32,
    eq_fn: EqFn,
    arena: i64,
) -> i64 {
    if depth >= HASH_LEVELS {
        // Collision level — node_ptr is a tagged collision or leaf.
        if is_collision(node_ptr) {
            return unsafe { get_collision(untag_collision(node_ptr), key, eq_fn, arena) };
        }
        // It's a leaf at collision level.
        let leaf_ptr = if is_leaf(node_ptr) {
            untag_leaf(node_ptr)
        } else {
            node_ptr
        };
        let existing_key = unsafe { read_kvpair_key(leaf_ptr) };
        if eq_fn(existing_key, key) == 1 {
            let value = unsafe { read_kvpair_value(leaf_ptr) };
            crate::sum::kata_rt_store_sum_result(0, value, arena)
        } else {
            crate::sum::kata_rt_store_sum_result(1, 0, arena)
        }
    } else {
        let bitmap = unsafe { read_bitmap(node_ptr) };
        let idx = hash_index(hash, depth);
        let bit = 1u32 << idx;

        if bitmap & bit == 0 {
            // Not found.
            crate::sum::kata_rt_store_sum_result(1, 0, arena)
        } else {
            let pos = child_index(bitmap, idx);
            let child = unsafe { read_child(node_ptr, pos) };

            if is_collision(child) {
                unsafe { get_collision(untag_collision(child), key, eq_fn, arena) }
            } else if is_leaf(child) {
                let leaf_ptr = untag_leaf(child);
                let existing_key = unsafe { read_kvpair_key(leaf_ptr) };
                if eq_fn(existing_key, key) == 1 {
                    let value = unsafe { read_kvpair_value(leaf_ptr) };
                    crate::sum::kata_rt_store_sum_result(0, value, arena)
                } else {
                    crate::sum::kata_rt_store_sum_result(1, 0, arena)
                }
            } else {
                // Interior node — recurse.
                unsafe { get_recursive(child, key, hash, depth + 1, eq_fn, arena) }
            }
        }
    }
}

/// Get from a collision node. Returns Result box.
unsafe fn get_collision(node_ptr: i64, key: i64, eq_fn: EqFn, arena: i64) -> i64 {
    let count = unsafe { read_collision_count(node_ptr) };
    for i in 0..count {
        let entry_ptr = unsafe { read_collision_entry(node_ptr, i) };
        let existing_key = unsafe { read_kvpair_key(entry_ptr) };
        if eq_fn(existing_key, key) == 1 {
            let value = unsafe { read_kvpair_value(entry_ptr) };
            return crate::sum::kata_rt_store_sum_result(0, value, arena);
        }
    }
    crate::sum::kata_rt_store_sum_result(1, 0, arena)
}

/// Recursive contains. Returns 1 or 0 (no Result box).
unsafe fn contains_recursive(node_ptr: i64, key: i64, hash: i64, depth: u32, eq_fn: EqFn) -> i64 {
    if depth >= HASH_LEVELS {
        // Collision level.
        if is_collision(node_ptr) {
            return unsafe { contains_collision(untag_collision(node_ptr), key, eq_fn) };
        }
        // It's a leaf at collision level.
        let leaf_ptr = if is_leaf(node_ptr) {
            untag_leaf(node_ptr)
        } else {
            node_ptr
        };
        let existing_key = unsafe { read_kvpair_key(leaf_ptr) };
        if eq_fn(existing_key, key) == 1 { 1 } else { 0 }
    } else {
        let bitmap = unsafe { read_bitmap(node_ptr) };
        let idx = hash_index(hash, depth);
        let bit = 1u32 << idx;

        if bitmap & bit == 0 {
            0
        } else {
            let pos = child_index(bitmap, idx);
            let child = unsafe { read_child(node_ptr, pos) };

            if is_collision(child) {
                unsafe { contains_collision(untag_collision(child), key, eq_fn) }
            } else if is_leaf(child) {
                let leaf_ptr = untag_leaf(child);
                let existing_key = unsafe { read_kvpair_key(leaf_ptr) };
                if eq_fn(existing_key, key) == 1 { 1 } else { 0 }
            } else {
                // Interior node — recurse.
                unsafe { contains_recursive(child, key, hash, depth + 1, eq_fn) }
            }
        }
    }
}

/// Contains check on a collision node. Returns 1 or 0.
unsafe fn contains_collision(node_ptr: i64, key: i64, eq_fn: EqFn) -> i64 {
    let count = unsafe { read_collision_count(node_ptr) };
    for i in 0..count {
        let entry_ptr = unsafe { read_collision_entry(node_ptr, i) };
        let existing_key = unsafe { read_kvpair_key(entry_ptr) };
        if eq_fn(existing_key, key) == 1 {
            return 1;
        }
    }
    0
}

/// Recursive count of entries.
unsafe fn count_recursive(node_ptr: i64) -> i64 {
    let bitmap = unsafe { read_bitmap(node_ptr) };
    let n = popcount(bitmap);
    let mut count = 0i64;
    for i in 0..n {
        let child = unsafe { read_child(node_ptr, i) };
        if is_collision(child) {
            let c = unsafe { read_collision_count(untag_collision(child)) };
            count += c;
        } else if is_leaf(child) {
            count += 1;
        } else {
            count += unsafe { count_recursive(child) };
        }
    }
    count
}

// ── Remove ───────────────────────────────────────────────

/// Recursive remove with copy-on-write.
///
/// Returns a new node pointer. If the key was not found, the returned node
/// is a copy identical to the original (COW still copies). The caller can
/// detect "not found" by comparing len before/after.
unsafe fn remove_recursive(
    node_ptr: i64,
    key: i64,
    hash: i64,
    depth: u32,
    eq_fn: EqFn,
    arena: i64,
) -> i64 {
    if depth >= HASH_LEVELS {
        // Collision level — node_ptr is a tagged collision or leaf.
        return unsafe { remove_at_collision_level(node_ptr, key, eq_fn, arena) };
    }

    let bitmap = unsafe { read_bitmap(node_ptr) };
    let idx = hash_index(hash, depth);
    let bit = 1u32 << idx;

    if bitmap & bit == 0 {
        // Key not present — return a copy of this node (unchanged).
        let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
        alloc_node(bitmap, &children, arena)
    } else {
        let pos = child_index(bitmap, idx);
        let child = unsafe { read_child(node_ptr, pos) };

        if is_collision(child) {
            // Collision node — try to remove from it.
            let coll_ptr = untag_collision(child);
            let new_coll = unsafe { remove_from_collision(coll_ptr, key, eq_fn, arena) };
            let new_count = unsafe { read_collision_count(new_coll) };

            let new_child = if new_count == 1 {
                // Collision shrank to 1 entry — promote it back to a leaf.
                let entry = unsafe { read_collision_entry(new_coll, 0) };
                tag_leaf(entry)
            } else {
                tag_collision(new_coll)
            };

            let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
            let mut new_children = children;
            new_children[pos] = new_child;
            alloc_node(bitmap, &new_children, arena)
        } else if is_leaf(child) {
            let leaf_ptr = untag_leaf(child);
            let existing_key = unsafe { read_kvpair_key(leaf_ptr) };

            if eq_fn(existing_key, key) == 1 {
                // Found — remove this leaf.
                let new_bitmap = bitmap & !bit;
                let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };

                if popcount(new_bitmap) == 0 {
                    // Node is now empty — return empty node.
                    return alloc_node(0, &[], arena);
                }

                // Remove the child at `pos` from the dense array.
                let mut new_children: Vec<i64> = Vec::with_capacity(children.len() - 1);
                for (i, &c) in children.iter().enumerate() {
                    if i != pos {
                        new_children.push(c);
                    }
                }
                alloc_node(new_bitmap, &new_children, arena)
            } else {
                // Key doesn't match — return a copy unchanged.
                let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
                alloc_node(bitmap, &children, arena)
            }
        } else {
            // Interior node — recurse.
            let new_child = unsafe { remove_recursive(child, key, hash, depth + 1, eq_fn, arena) };
            let children = unsafe { read_all_children(node_ptr, popcount(bitmap)) };
            let mut new_children = children;
            new_children[pos] = new_child;
            alloc_node(bitmap, &new_children, arena)
        }
    }
}

/// Remove from a collision-level node. Returns a new collision node (untagged).
/// If the key is not found, returns a copy of the original.
unsafe fn remove_at_collision_level(node_ptr: i64, key: i64, eq_fn: EqFn, arena: i64) -> i64 {
    if is_collision(node_ptr) {
        let coll_ptr = untag_collision(node_ptr);
        return unsafe { remove_from_collision(coll_ptr, key, eq_fn, arena) };
    }

    // It's a leaf at collision level.
    let leaf_ptr = if is_leaf(node_ptr) {
        untag_leaf(node_ptr)
    } else {
        node_ptr
    };
    let existing_key = unsafe { read_kvpair_key(leaf_ptr) };

    if eq_fn(existing_key, key) == 1 {
        // Remove the only entry — return empty node.
        alloc_node(0, &[], arena)
    } else {
        // Key doesn't match — return a copy (collision node wrapping this leaf).
        // Actually, at collision level a single leaf is just a leaf.
        // Return it as-is (tagged leaf) — but we need to match the caller's expectation.
        // The caller (remove_recursive at depth >= HASH_LEVELS) gets back a pointer
        // that is used as the new root. We return the tagged leaf.
        tag_leaf(leaf_ptr)
    }
}

/// Remove an entry from a collision node. Returns a new collision node (untagged).
/// If the key is not found, returns a copy of the original.
unsafe fn remove_from_collision(coll_ptr: i64, key: i64, eq_fn: EqFn, arena: i64) -> i64 {
    let count = unsafe { read_collision_count(coll_ptr) };

    for i in 0..count {
        let entry_ptr = unsafe { read_collision_entry(coll_ptr, i) };
        let existing_key = unsafe { read_kvpair_key(entry_ptr) };
        if eq_fn(existing_key, key) == 1 {
            // Found — remove this entry.
            let new_count = count - 1;
            if new_count == 0 {
                return alloc_node(0, &[], arena);
            }
            let mut entries: Vec<i64> = Vec::with_capacity(new_count as usize);
            for j in 0..count {
                if j != i {
                    entries.push(unsafe { read_collision_entry(coll_ptr, j) });
                }
            }
            return alloc_collision(new_count, &entries, arena);
        }
    }

    // Key not found — return a copy.
    let mut entries: Vec<i64> = Vec::with_capacity(count as usize);
    for j in 0..count {
        entries.push(unsafe { read_collision_entry(coll_ptr, j) });
    }
    alloc_collision(count, &entries, arena)
}

// ── HAMT iteration helpers ──────────────────────────────────

/// Recursive helper for collect_all_kvpairs.
unsafe fn collect_recursive(node_ptr: i64, out: &mut Vec<i64>) {
    // Check if this is a tagged child (leaf or collision) — can happen at depth 0
    // if the root itself is a leaf/collision (shouldn't normally, but be safe).
    if is_collision(node_ptr) {
        let coll_ptr = untag_collision(node_ptr);
        let count = unsafe { read_collision_count(coll_ptr) };
        for i in 0..count {
            let entry = unsafe { read_collision_entry(coll_ptr, i) };
            out.push(entry);
        }
        return;
    }
    if is_leaf(node_ptr) {
        out.push(untag_leaf(node_ptr));
        return;
    }

    // Interior node.
    let bitmap = unsafe { read_bitmap(node_ptr) };
    let n = popcount(bitmap);
    for i in 0..n {
        let child = unsafe { read_child(node_ptr, i) };
        if is_collision(child) {
            let coll_ptr = untag_collision(child);
            let count = unsafe { read_collision_count(coll_ptr) };
            for j in 0..count {
                let entry = unsafe { read_collision_entry(coll_ptr, j) };
                out.push(entry);
            }
        } else if is_leaf(child) {
            out.push(untag_leaf(child));
        } else {
            unsafe { collect_recursive(child, out) };
        }
    }
}

/// Allocate a 16-byte tuple (key, value) in the arena from KVPair at
/// `arr[index]` and return a Some Sum box pointing to it.
unsafe fn make_kv_tuple(arr: i64, index: i64, arena_handle: i64) -> i64 {
    let kvptr = unsafe {
        std::ptr::read_unaligned((arr as *const u8).add((index as usize) * 8) as *const i64)
    };
    let key = unsafe { read_kvpair_key(kvptr) };
    let value = unsafe { read_kvpair_value(kvptr) };

    // Allocate 16-byte tuple: key at 0, value at 8.
    let tuple = crate::arena::kata_rt_arena_alloc(arena_handle, 16);
    if tuple == 0 {
        return crate::sum::kata_rt_store_sum_result(1, 0, arena_handle);
    }
    unsafe {
        std::ptr::write_unaligned(tuple as *mut i64, key);
        std::ptr::write_unaligned((tuple as *mut u8).add(8) as *mut i64, value);
    }
    // Return Some(tuple) — tag=0, payload=tuple pointer.
    crate::sum::kata_rt_store_sum_result(0, tuple, arena_handle)
}

// ── Thread-local iterator state (HAMT-order, used by hamt_next) ──

use std::cell::Cell;

thread_local! {
    static ITER_ARRAY: Cell<i64> = const { Cell::new(0) };
    static ITER_COUNT: Cell<i64> = const { Cell::new(0) };
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
    let ptr = crate::arena::kata_rt_arena_alloc(arena_handle, 16);
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
    let cons_cell = crate::arena::kata_rt_arena_alloc(arena_handle, 16);
    if cons_cell == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(cons_cell as *mut i64, kvpair_ptr); // head
        std::ptr::write_unaligned((cons_cell as *mut u8).add(8) as *mut i64, old_log); // tail
    }
    let new_log = cons_cell;

    // 3. Allocate new Dict struct (16 bytes) with (new_hamt, new_log)
    let new_dict = crate::arena::kata_rt_arena_alloc(arena_handle, 16);
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
    let new_dict = crate::arena::kata_rt_arena_alloc(arena_handle, 16);
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
        let arr = crate::arena::kata_rt_arena_alloc(arena_handle, arr_size);
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

/// Walk the HAMT to find the leaf at the given hash and compare the KVPair
/// pointer. Returns true if the KVPair pointer matches (i.e., the entry is
/// still current, not replaced or removed). This avoids needing eq_fn.
unsafe fn hamt_find_kvpair(node_ptr: i64, hash: i64, target_kvpair: i64, depth: u32) -> bool {
    if depth >= HASH_LEVELS {
        // Collision level.
        if is_collision(node_ptr) {
            let coll_ptr = untag_collision(node_ptr);
            let count = unsafe { read_collision_count(coll_ptr) };
            for i in 0..count {
                let entry = unsafe { read_collision_entry(coll_ptr, i) };
                if entry == target_kvpair {
                    return true;
                }
            }
            return false;
        }
        // Leaf at collision level.
        let leaf_ptr = if is_leaf(node_ptr) {
            untag_leaf(node_ptr)
        } else {
            node_ptr
        };
        return leaf_ptr == target_kvpair;
    }

    // Interior node.
    if is_collision(node_ptr) {
        let coll_ptr = untag_collision(node_ptr);
        let count = unsafe { read_collision_count(coll_ptr) };
        for i in 0..count {
            let entry = unsafe { read_collision_entry(coll_ptr, i) };
            if entry == target_kvpair {
                return true;
            }
        }
        return false;
    }
    if is_leaf(node_ptr) {
        return untag_leaf(node_ptr) == target_kvpair;
    }

    let bitmap = unsafe { read_bitmap(node_ptr) };
    let idx = hash_index(hash, depth);
    let bit = 1u32 << idx;

    if bitmap & bit == 0 {
        return false;
    }

    let pos = child_index(bitmap, idx);
    let child = unsafe { read_child(node_ptr, pos) };

    if is_collision(child) {
        let coll_ptr = untag_collision(child);
        let count = unsafe { read_collision_count(coll_ptr) };
        for i in 0..count {
            let entry = unsafe { read_collision_entry(coll_ptr, i) };
            if entry == target_kvpair {
                return true;
            }
        }
        false
    } else if is_leaf(child) {
        untag_leaf(child) == target_kvpair
    } else {
        unsafe { hamt_find_kvpair(child, hash, target_kvpair, depth + 1) }
    }
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
pub extern "C" fn kata_rt_dict_merge(
    a: i64,
    b: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
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

    fn smi(n: i64) -> i64 { (n << 1) | 1 }

    #[test]
    fn test_hamt_two_int_inserts() {
        let arena = crate::arena::kata_rt_arena_create();
        let dict = kata_rt_dict_empty(arena);
        let len0 = kata_rt_dict_len(dict);
        assert_eq!(len0, smi(0), "empty dict len should be SMI(0), got {}", len0);

        let key1 = 3i64;
        let val1 = 21i64;
        let hash1 = crate::hash::kata_rt_hash_int(key1);
        let eq_fn = crate::bigint::kata_rt_bi_eq as *const () as i64;
        let dict1 = kata_rt_dict_insert(dict, key1, val1, hash1, eq_fn, arena);
        let len1 = kata_rt_dict_len(dict1);
        assert_eq!(len1, smi(1), "after 1 insert, len should be SMI(1), got {}", len1);

        let key2 = 5i64;
        let val2 = 41i64;
        let hash2 = crate::hash::kata_rt_hash_int(key2);
        let dict2 = kata_rt_dict_insert(dict1, key2, val2, hash2, eq_fn, arena);
        let len2 = kata_rt_dict_len(dict2);
        assert_eq!(len2, smi(2), "after 2 inserts, len should be SMI(2), got {}", len2);

        let key3 = 7i64;
        let val3 = 61i64;
        let hash3 = crate::hash::kata_rt_hash_int(key3);
        let dict3 = kata_rt_dict_insert(dict2, key3, val3, hash3, eq_fn, arena);
        let len3 = kata_rt_dict_len(dict3);
        assert_eq!(len3, smi(3), "after 3 inserts, len should be SMI(3), got {}", len3);
    }

    #[test]
    fn test_hamt_text_keys() {
        let arena = crate::arena::kata_rt_arena_create();
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
        assert_eq!(len2, smi(2), "text-keyed dict with 2 entries should have len=SMI(2)");
    }
}
