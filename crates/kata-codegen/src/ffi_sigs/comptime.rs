//! Assinaturas FFI para comptime snapshots e cache (@cache).

use crate::call_conv::ffi_call_conv;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de snapshots e cache.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(ffi_call_conv());
    match sym {
        // ── Comptime snapshots (Fio 12) ──
        // load_snapshot: (root_arena, bytes_ptr, bytes_len, rebase_offsets_ptr, rebase_count, snapshot_id) -> ()
        FfiSymbol::LoadSnapshot => {
            sig.params.push(AbiParam::new(I64)); // root_arena
            sig.params.push(AbiParam::new(I64)); // bytes_ptr
            sig.params.push(AbiParam::new(I64)); // bytes_len
            sig.params.push(AbiParam::new(I64)); // rebase_offsets_ptr
            sig.params.push(AbiParam::new(I64)); // rebase_count
            sig.params.push(AbiParam::new(I64)); // snapshot_id
        }
        // get_snapshot: (snapshot_id) -> ptr
        FfiSymbol::GetSnapshot => {
            sig.params.push(AbiParam::new(I64)); // snapshot_id
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // ── Cache @cache{strategy: "LRU"} (Fio 12, Fase 5) ──
        // cache_get_or_create: (arena, fn_id, capacity) -> handle
        FfiSymbol::CacheGetOrCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // fn_id
            sig.params.push(AbiParam::new(I64)); // capacity
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // cache_lookup: (handle, key_ptr, key_len) -> i64 (0=miss, ptr=hit)
        FfiSymbol::CacheLookup => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // key_ptr
            sig.params.push(AbiParam::new(I64)); // key_len
            sig.returns.push(AbiParam::new(I64)); // value (0=miss, ptr=hit)
        }
        // cache_insert: (handle, key_ptr, key_len, value) -> ()
        FfiSymbol::CacheInsert => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // key_ptr
            sig.params.push(AbiParam::new(I64)); // key_len
            sig.params.push(AbiParam::new(I64)); // value
        }
        // cache_serialize_key: (value, desc_ptr, desc_len, out_ptr, out_cap) -> i64
        FfiSymbol::CacheSerializeKey => {
            sig.params.push(AbiParam::new(I64)); // value
            sig.params.push(AbiParam::new(I64)); // desc_ptr
            sig.params.push(AbiParam::new(I64)); // desc_len
            sig.params.push(AbiParam::new(I64)); // out_ptr
            sig.params.push(AbiParam::new(I64)); // out_cap
            sig.returns.push(AbiParam::new(I64)); // bytes_written (-1 = error)
        }
        _ => return None,
    }
    Some(sig)
}
