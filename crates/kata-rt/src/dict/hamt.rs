//! HAMT recursive algorithm and low-level helpers.
//!
//! Contains pointer-tagging helpers, arena allocators, node readers,
//! recursive insert/get/remove/collect operations, and the `pub(super)`
//! internal API wrappers used by the FFI layer in `dict/mod.rs`.

use super::{COLLISION_SENTINEL, COLLISION_TAG, EqFn, HASH_BITS, HASH_LEVELS, HASH_MASK, LEAF_TAG};

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
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena, 24);
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
pub(super) unsafe fn read_kvpair_key(leaf_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned(leaf_ptr as *const i64) }
}

/// Read the value from a KVPair.
pub(super) unsafe fn read_kvpair_value(leaf_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned((leaf_ptr as *const u8).add(8) as *const i64) }
}

/// Read the hash from a KVPair.
pub(super) unsafe fn read_kvpair_hash(leaf_ptr: i64) -> i64 {
    unsafe { std::ptr::read_unaligned((leaf_ptr as *const u8).add(16) as *const i64) }
}

/// Allocate an interior node with the given bitmap and children.
fn alloc_node(bitmap: u32, children: &[i64], arena: i64) -> i64 {
    let header_size = 8; // bitmap as i64
    let children_size = children.len() * 8;
    let total = header_size + children_size;
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena, total as i64);
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
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena, total as i64);
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
// Internal HAMT functions (pub(super)) — operate on raw HAMT root
// ════════════════════════════════════════════════════════════════

/// Allocate an empty HAMT root node (bitmap = 0, no children).
///
/// Returns a pointer to the root node.
pub(super) fn hamt_empty(arena_handle: i64) -> i64 {
    // Root node: 8 bytes (bitmap=0 as i64). No children.
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 8);
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
pub(super) fn hamt_insert(
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
pub(super) fn hamt_get_checked(
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
pub(super) fn hamt_contains(hamt_root: i64, key: i64, hash: i64, eq_fn: i64) -> i64 {
    let eq = unsafe { std::mem::transmute::<i64, EqFn>(eq_fn) };
    unsafe { contains_recursive(hamt_root, key, hash, 0, eq) }
}

/// Count entries by traversing the trie. O(n).
pub(super) fn hamt_len(hamt_root: i64) -> i64 {
    unsafe { count_recursive(hamt_root) }
}

/// Remove `key` from the HAMT. Returns a NEW root pointer (original unchanged).
pub(super) fn hamt_remove(
    hamt_root: i64,
    key: i64,
    hash: i64,
    eq_fn: i64,
    arena_handle: i64,
) -> i64 {
    let eq = unsafe { std::mem::transmute::<i64, EqFn>(eq_fn) };
    unsafe { remove_recursive(hamt_root, key, hash, 0, eq, arena_handle) }
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
        let arr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 8);
        if arr != 0 {
            unsafe { std::ptr::write_unaligned(arr as *mut i64, 0) };
        }
        return (arr, 0);
    }

    // Allocate array in arena: count * 8 bytes.
    let arr_size = count * 8;
    let arr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, arr_size);
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
pub(super) unsafe fn make_kv_tuple(arr: i64, index: i64, arena_handle: i64) -> i64 {
    let kvptr = unsafe {
        std::ptr::read_unaligned((arr as *const u8).add((index as usize) * 8) as *const i64)
    };
    let key = unsafe { read_kvpair_key(kvptr) };
    let value = unsafe { read_kvpair_value(kvptr) };

    // Allocate 16-byte tuple: key at 0, value at 8.
    let tuple = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, 16);
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

/// Walk the HAMT to find the leaf at the given hash and compare the KVPair
/// pointer. Returns true if the KVPair pointer matches (i.e., the entry is
/// still current, not replaced or removed). This avoids needing eq_fn.
pub(super) unsafe fn hamt_find_kvpair(
    node_ptr: i64,
    hash: i64,
    target_kvpair: i64,
    depth: u32,
) -> bool {
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
