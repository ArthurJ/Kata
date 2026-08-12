//! Assinaturas FFI para collections: List, Array, Range, Hash, Dict, Set.
//!
//! Inclui operações de construção, acesso, iteração, slices e merges.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use crate::call_conv::ffi_call_conv;
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de collections.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(ffi_call_conv());
    match sym {
        // ── Collections ──
        // list_nil: () -> ptr (0 = null)
        FfiSymbol::ListNil => {
            sig.returns.push(AbiParam::new(I64));
        }
        // list_cons: (head, tail, arena) -> ptr
        FfiSymbol::ListCons => {
            sig.params.push(AbiParam::new(I64)); // head
            sig.params.push(AbiParam::new(I64)); // tail
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // list_is_empty: (ptr) -> i64 (0/1)
        FfiSymbol::ListIsEmpty => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // bool
        }
        // list_head: (ptr) -> i64
        FfiSymbol::ListHead => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // head
        }
        // list_tail: (ptr) -> ptr
        FfiSymbol::ListTail => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // tail
        }
        // list_len: (ptr) -> i64
        FfiSymbol::ListLen => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // len
        }
        // list_get_checked: (ptr, idx) -> ptr (Result box)
        FfiSymbol::ListGetChecked => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // array_alloc: (len, arena) -> ptr
        FfiSymbol::ArrayAlloc => {
            sig.params.push(AbiParam::new(I64)); // len
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // array_len: (ptr) -> i64
        FfiSymbol::ArrayLen => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // len
        }
        // array_get: (ptr, idx) -> i64
        FfiSymbol::ArrayGet => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // array_set: (ptr, idx, val) -> void
        FfiSymbol::ArraySet => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.params.push(AbiParam::new(I64)); // val
        }
        // array_get_checked: (ptr, idx) -> ptr (Result box)
        FfiSymbol::ArrayGetChecked => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // range_alloc: (arena) -> ptr
        FfiSymbol::RangeAlloc => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // list_contains: (ptr, item) -> i64 (0/1)
        FfiSymbol::ListContains => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // item
            sig.returns.push(AbiParam::new(I64)); // bool
        }
        // array_contains: (ptr, item) -> i64 (0/1)
        FfiSymbol::ArrayContains => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // item
            sig.returns.push(AbiParam::new(I64)); // bool
        }
        // list_reverse: (ptr, arena) -> ptr (inverte Cons chain)
        FfiSymbol::ListReverse => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr (reversed list)
        }
        // list_concat: (first, second, arena) -> ptr (concatena duas listas)
        FfiSymbol::ListConcat => {
            sig.params.push(AbiParam::new(I64)); // first
            sig.params.push(AbiParam::new(I64)); // second
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr (concatenated list)
        }
        // ── Hash (Fio 13) ──
        // hash_int: (val) -> i64 (hash)
        FfiSymbol::HashInt => {
            sig.params.push(AbiParam::new(I64)); // val (SMI-tagged)
            sig.returns.push(AbiParam::new(I64)); // hash
        }
        // hash_text: (str_ptr) -> i64 (hash)
        FfiSymbol::HashText => {
            sig.params.push(AbiParam::new(I64)); // str_ptr
            sig.returns.push(AbiParam::new(I64)); // hash
        }
        // hash_rational: (rat_ptr) -> i64 (hash)
        FfiSymbol::HashRational => {
            sig.params.push(AbiParam::new(I64)); // rat_ptr
            sig.returns.push(AbiParam::new(I64)); // hash
        }
        // ── Dict (Fio 13) ──
        // dict_empty: (arena) -> i64 (dict ptr)
        FfiSymbol::DictEmpty => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // dict ptr
        }
        // dict_insert: (dict, key, val, hash, eq_fn, arena) -> i64 (new dict ptr)
        FfiSymbol::DictInsert => {
            sig.params.push(AbiParam::new(I64)); // dict
            sig.params.push(AbiParam::new(I64)); // key
            sig.params.push(AbiParam::new(I64)); // value
            sig.params.push(AbiParam::new(I64)); // hash
            sig.params.push(AbiParam::new(I64)); // eq_fn (function pointer)
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // new dict ptr
        }
        // dict_get_checked: (dict, key, hash, eq_fn, arena) -> i64 (Result box)
        FfiSymbol::DictGetChecked => {
            sig.params.push(AbiParam::new(I64)); // dict
            sig.params.push(AbiParam::new(I64)); // key
            sig.params.push(AbiParam::new(I64)); // hash
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // dict_contains: (dict, key, hash, eq_fn) -> i64 (0/1)
        FfiSymbol::DictContains => {
            sig.params.push(AbiParam::new(I64)); // dict
            sig.params.push(AbiParam::new(I64)); // key
            sig.params.push(AbiParam::new(I64)); // hash
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.returns.push(AbiParam::new(I64)); // bool (0/1)
        }
        // dict_len: (dict) -> i64
        FfiSymbol::DictLen => {
            sig.params.push(AbiParam::new(I64)); // dict
            sig.returns.push(AbiParam::new(I64)); // count
        }
        // dict_remove: (dict, key, hash, eq_fn, arena) -> i64 (new dict ptr)
        FfiSymbol::DictRemove => {
            sig.params.push(AbiParam::new(I64)); // dict
            sig.params.push(AbiParam::new(I64)); // key
            sig.params.push(AbiParam::new(I64)); // hash
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // new dict ptr
        }
        // dict_next: (dict, iter_state, arena) -> i64 (Optional box)
        FfiSymbol::DictNext => {
            sig.params.push(AbiParam::new(I64)); // dict
            sig.params.push(AbiParam::new(I64)); // iter_state
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // Optional box
        }
        // ── Set (Fio 13) ──
        // set_empty: (arena) -> i64 (set ptr)
        FfiSymbol::SetEmpty => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // set ptr
        }
        // set_insert: (set, elem, hash, eq_fn, arena) -> i64 (new set ptr)
        FfiSymbol::SetInsert => {
            sig.params.push(AbiParam::new(I64)); // set
            sig.params.push(AbiParam::new(I64)); // elem
            sig.params.push(AbiParam::new(I64)); // hash
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // new set ptr
        }
        // set_contains: (set, elem, hash, eq_fn) -> i64 (0/1)
        FfiSymbol::SetContains => {
            sig.params.push(AbiParam::new(I64)); // set
            sig.params.push(AbiParam::new(I64)); // elem
            sig.params.push(AbiParam::new(I64)); // hash
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.returns.push(AbiParam::new(I64)); // bool (0/1)
        }
        // set_len: (set) -> i64
        FfiSymbol::SetLen => {
            sig.params.push(AbiParam::new(I64)); // set
            sig.returns.push(AbiParam::new(I64)); // count
        }
        // set_remove: (set, elem, hash, eq_fn, arena) -> i64 (new set ptr)
        FfiSymbol::SetRemove => {
            sig.params.push(AbiParam::new(I64)); // set
            sig.params.push(AbiParam::new(I64)); // elem
            sig.params.push(AbiParam::new(I64)); // hash
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // new set ptr
        }
        // set_next: (set, iter_state, arena) -> i64 (Optional box)
        FfiSymbol::SetNext => {
            sig.params.push(AbiParam::new(I64)); // set
            sig.params.push(AbiParam::new(I64)); // iter_state
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // Optional box
        }
        // set_union: (a, b, eq_fn, arena) -> i64 (new set ptr)
        FfiSymbol::SetUnion => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // new set ptr
        }
        // set_intersection: (a, b, eq_fn, arena) -> i64 (new set ptr)
        FfiSymbol::SetIntersection => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // new set ptr
        }
        // set_difference: (a, b, eq_fn, arena) -> i64 (new set ptr)
        FfiSymbol::SetDifference => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // new set ptr
        }
        // dict_merge: (a, b, eq_fn, arena) -> i64 (new dict ptr)
        FfiSymbol::DictMerge => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // eq_fn
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // new dict ptr
        }
        // ── Collections slices ──
        // array_slice: (ptr, start, end, arena) -> ptr
        FfiSymbol::ArraySlice => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // start
            sig.params.push(AbiParam::new(I64)); // end
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // list_slice: (ptr, start, end, arena) -> ptr
        FfiSymbol::ListSlice => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // start
            sig.params.push(AbiParam::new(I64)); // end
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        _ => return None,
    }
    Some(sig)
}
