//! Registro e declaração de símbolos FFI no JIT.
//!
//! `register_ffi_symbols` popula o `JITBuilder` com os ponteiros das funções
//! C do `kata-rt`. `declare_ffi_symbols` declara os imports no `JITModule`
//! e retorna o mapa nome → FuncId.

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_module::Linkage;
use kata_core::ffi::FfiSymbol;
use kata_rt as rt;

use crate::ffi_sigs::ffi_signature;
use crate::lowering::CodegenError;
use crate::lowering::ModuleBackend;

/// Registro de símbolos FFI no JITBuilder.
///
/// Cada símbolo do `FfiSymbol` enum é registrado com o ponteiro da função
/// C correspondente no `kata-rt`. O JIT usa esta tabela para resolver
/// imports.
pub(crate) fn register_ffi_symbols(builder: &mut cranelift_jit::JITBuilder) {
    // BigInt
    builder.symbol("kata_rt_bi_add", rt::kata_rt_bi_add as *const u8);
    builder.symbol("kata_rt_bi_sub", rt::kata_rt_bi_sub as *const u8);
    builder.symbol("kata_rt_bi_mul", rt::kata_rt_bi_mul as *const u8);
    builder.symbol("kata_rt_bi_div", rt::kata_rt_bi_div as *const u8);
    builder.symbol("kata_rt_bi_eq", rt::kata_rt_bi_eq as *const u8);
    builder.symbol("kata_rt_bi_neq", rt::kata_rt_bi_neq as *const u8);
    builder.symbol("kata_rt_bi_lt", rt::kata_rt_bi_lt as *const u8);
    builder.symbol("kata_rt_bi_le", rt::kata_rt_bi_le as *const u8);
    builder.symbol("kata_rt_bi_gt", rt::kata_rt_bi_gt as *const u8);
    builder.symbol("kata_rt_bi_ge", rt::kata_rt_bi_ge as *const u8);
    builder.symbol("kata_rt_bi_show", rt::kata_rt_bi_show as *const u8);
    builder.symbol(
        "kata_rt_bi_to_rational",
        rt::kata_rt_bi_to_rational as *const u8,
    );
    builder.symbol("kata_rt_tag_int", rt::kata_rt_tag_int as *const u8);
    builder.symbol(
        "kata_rt_tag_int_from_str",
        rt::kata_rt_tag_int_from_str as *const u8,
    );
    builder.symbol("kata_rt_int_to_text", rt::kata_rt_int_to_text as *const u8);
    // Float
    builder.symbol("kata_rt_fadd", rt::kata_rt_fadd as *const u8);
    builder.symbol("kata_rt_fsub", rt::kata_rt_fsub as *const u8);
    builder.symbol("kata_rt_fmul", rt::kata_rt_fmul as *const u8);
    builder.symbol("kata_rt_fdiv", rt::kata_rt_fdiv as *const u8);
    builder.symbol("kata_rt_fcmp_eq", rt::kata_rt_fcmp_eq as *const u8);
    builder.symbol("kata_rt_fcmp_neq", rt::kata_rt_fcmp_neq as *const u8);
    builder.symbol("kata_rt_fcmp_lt", rt::kata_rt_fcmp_lt as *const u8);
    builder.symbol("kata_rt_fcmp_le", rt::kata_rt_fcmp_le as *const u8);
    builder.symbol("kata_rt_fcmp_gt", rt::kata_rt_fcmp_gt as *const u8);
    builder.symbol("kata_rt_fcmp_ge", rt::kata_rt_fcmp_ge as *const u8);
    builder.symbol(
        "kata_rt_float_to_text",
        rt::kata_rt_float_to_text as *const u8,
    );
    // Rational
    builder.symbol("kata_rt_rat_add", rt::kata_rt_rat_add as *const u8);
    builder.symbol("kata_rt_rat_sub", rt::kata_rt_rat_sub as *const u8);
    builder.symbol("kata_rt_rat_mul", rt::kata_rt_rat_mul as *const u8);
    builder.symbol("kata_rt_rat_div", rt::kata_rt_rat_div as *const u8);
    builder.symbol("kata_rt_rat_eq", rt::kata_rt_rat_eq as *const u8);
    builder.symbol("kata_rt_rat_neq", rt::kata_rt_rat_neq as *const u8);
    builder.symbol("kata_rt_rat_lt", rt::kata_rt_rat_lt as *const u8);
    builder.symbol("kata_rt_rat_le", rt::kata_rt_rat_le as *const u8);
    builder.symbol("kata_rt_rat_gt", rt::kata_rt_rat_gt as *const u8);
    builder.symbol("kata_rt_rat_ge", rt::kata_rt_rat_ge as *const u8);
    builder.symbol("kata_rt_rat_show", rt::kata_rt_rat_show as *const u8);
    builder.symbol(
        "kata_rt_rat_to_float",
        rt::kata_rt_rat_to_float as *const u8,
    );
    builder.symbol(
        "kata_rt_rat_from_float",
        rt::kata_rt_rat_from_float as *const u8,
    );
    builder.symbol("kata_rt_rat_literal", rt::kata_rt_rat_literal as *const u8);
    builder.symbol(
        "kata_rt_int_to_rational",
        rt::kata_rt_int_to_rational as *const u8,
    );
    // Text
    builder.symbol(
        "kata_rt_string_concat",
        rt::kata_rt_string_concat as *const u8,
    );
    builder.symbol("kata_rt_string_len", rt::kata_rt_string_len as *const u8);
    builder.symbol(
        "kata_rt_text_literal",
        rt::kata_rt_text_literal as *const u8,
    );
    builder.symbol(
        "kata_rt_bool_to_text",
        rt::kata_rt_bool_to_text as *const u8,
    );
    builder.symbol(
        "kata_rt_text_replace_first",
        rt::kata_rt_text_replace_first as *const u8,
    );
    // I/O
    builder.symbol("kata_rt_print", rt::kata_rt_print as *const u8);
    builder.symbol("kata_rt_println", rt::kata_rt_println as *const u8);
    // Control flow — panic
    builder.symbol("kata_rt_panic", rt::kata_rt_panic as *const u8);
    // Arena — C-ABI para alocação de tuplas (DoD 22)
    builder.symbol(
        "kata_rt_arena_create",
        rt::kata_rt_arena_create as *const u8,
    );
    builder.symbol("kata_rt_arena_alloc", rt::kata_rt_arena_alloc as *const u8);
    builder.symbol(
        "kata_rt_arena_destroy",
        rt::kata_rt_arena_destroy as *const u8,
    );
    // Arena Tracked — root arena para valores ARC-managed (Fio 16)
    builder.symbol(
        "kata_rt_arena_create_tracked",
        rt::kata_rt_arena_create_tracked as *const u8,
    );
    builder.symbol(
        "kata_rt_arena_dealloc",
        rt::kata_rt_arena_dealloc as *const u8,
    );
    builder.symbol(
        "kata_rt_get_root_arena_handle",
        rt::kata_rt_get_root_arena_handle as *const u8,
    );
    builder.symbol(
        "kata_rt_alloc_tracked",
        rt::kata_rt_alloc_tracked as *const u8,
    );
    builder.symbol(
        "kata_rt_incref_tracked",
        rt::kata_rt_incref_tracked as *const u8,
    );
    builder.symbol(
        "kata_rt_decref_tracked",
        rt::kata_rt_decref_tracked as *const u8,
    );
    // Sum
    builder.symbol(
        "kata_rt_store_sum_result",
        rt::kata_rt_store_sum_result as *const u8,
    );
    builder.symbol("kata_rt_sum_tag_int", rt::kata_rt_sum_tag_int as *const u8);
    // Scheduler/Fiber
    builder.symbol(
        "kata_rt_scheduler_init",
        rt::kata_rt_scheduler_init as *const u8,
    );
    builder.symbol("kata_rt_spawn", rt::kata_rt_spawn as *const u8);
    builder.symbol("kata_rt_run", rt::kata_rt_run as *const u8);
    builder.symbol("kata_rt_yield", rt::kata_rt_yield as *const u8);
    builder.symbol("kata_rt_yield_check", rt::kata_rt_yield_check as *const u8);
    builder.symbol(
        "kata_rt_set_test_timeout",
        rt::kata_rt_set_test_timeout as *const u8,
    );
    builder.symbol("kata_rt_sleep", rt::kata_rt_sleep as *const u8);
    // Arc<T> / CaptureBox
    builder.symbol("kata_rt_alloc_arc", rt::kata_rt_alloc_arc as *const u8);
    builder.symbol("kata_rt_incref", rt::kata_rt_incref as *const u8);
    builder.symbol("kata_rt_decref", rt::kata_rt_decref as *const u8);
    builder.symbol("kata_rt_arc_fn_ptr", rt::kata_rt_arc_fn_ptr as *const u8);
    // Collections
    builder.symbol("kata_rt_list_nil", rt::kata_rt_list_nil as *const u8);
    builder.symbol("kata_rt_list_cons", rt::kata_rt_list_cons as *const u8);
    builder.symbol(
        "kata_rt_list_is_empty",
        rt::kata_rt_list_is_empty as *const u8,
    );
    builder.symbol("kata_rt_list_head", rt::kata_rt_list_head as *const u8);
    builder.symbol("kata_rt_list_tail", rt::kata_rt_list_tail as *const u8);
    builder.symbol("kata_rt_list_len", rt::kata_rt_list_len as *const u8);
    builder.symbol(
        "kata_rt_list_get_checked",
        rt::kata_rt_list_get_checked as *const u8,
    );
    builder.symbol(
        "kata_rt_list_reverse",
        rt::kata_rt_list_reverse as *const u8,
    );
    builder.symbol("kata_rt_list_concat", rt::kata_rt_list_concat as *const u8);
    builder.symbol("kata_rt_array_alloc", rt::kata_rt_array_alloc as *const u8);
    builder.symbol("kata_rt_array_len", rt::kata_rt_array_len as *const u8);
    builder.symbol("kata_rt_array_get", rt::kata_rt_array_get as *const u8);
    builder.symbol("kata_rt_array_set", rt::kata_rt_array_set as *const u8);
    builder.symbol(
        "kata_rt_array_get_checked",
        rt::kata_rt_array_get_checked as *const u8,
    );
    builder.symbol("kata_rt_range_alloc", rt::kata_rt_range_alloc as *const u8);
    builder.symbol(
        "kata_rt_list_contains",
        rt::kata_rt_list_contains as *const u8,
    );
    builder.symbol(
        "kata_rt_array_contains",
        rt::kata_rt_array_contains as *const u8,
    );
    // Hash (Fio 13)
    builder.symbol("kata_rt_hash_int", rt::kata_rt_hash_int as *const u8);
    builder.symbol("kata_rt_hash_text", rt::kata_rt_hash_text as *const u8);
    builder.symbol(
        "kata_rt_hash_rational",
        rt::kata_rt_hash_rational as *const u8,
    );
    // Dict (Fio 13)
    builder.symbol("kata_rt_dict_empty", rt::kata_rt_dict_empty as *const u8);
    builder.symbol("kata_rt_dict_insert", rt::kata_rt_dict_insert as *const u8);
    builder.symbol(
        "kata_rt_dict_get_checked",
        rt::kata_rt_dict_get_checked as *const u8,
    );
    builder.symbol(
        "kata_rt_dict_contains",
        rt::kata_rt_dict_contains as *const u8,
    );
    builder.symbol("kata_rt_dict_len", rt::kata_rt_dict_len as *const u8);
    builder.symbol("kata_rt_dict_remove", rt::kata_rt_dict_remove as *const u8);
    builder.symbol("kata_rt_dict_next", rt::kata_rt_dict_next as *const u8);
    // Set (Fio 13)
    builder.symbol("kata_rt_set_empty", rt::kata_rt_set_empty as *const u8);
    builder.symbol("kata_rt_set_insert", rt::kata_rt_set_insert as *const u8);
    builder.symbol(
        "kata_rt_set_contains",
        rt::kata_rt_set_contains as *const u8,
    );
    builder.symbol("kata_rt_set_len", rt::kata_rt_set_len as *const u8);
    builder.symbol("kata_rt_set_remove", rt::kata_rt_set_remove as *const u8);
    builder.symbol("kata_rt_set_next", rt::kata_rt_set_next as *const u8);
    builder.symbol("kata_rt_set_union", rt::kata_rt_set_union as *const u8);
    builder.symbol(
        "kata_rt_set_intersection",
        rt::kata_rt_set_intersection as *const u8,
    );
    builder.symbol(
        "kata_rt_set_difference",
        rt::kata_rt_set_difference as *const u8,
    );
    builder.symbol("kata_rt_dict_merge", rt::kata_rt_dict_merge as *const u8);
    // String equality (for Text keys in Dict/Set)
    builder.symbol("kata_rt_string_eq", rt::kata_rt_string_eq as *const u8);
    // Canais CSP
    builder.symbol(
        "kata_rt_channel_create",
        rt::kata_rt_channel_create as *const u8,
    );
    builder.symbol(
        "kata_rt_queue_create",
        rt::kata_rt_queue_create as *const u8,
    );
    builder.symbol(
        "kata_rt_broadcast_create",
        rt::kata_rt_broadcast_create as *const u8,
    );
    builder.symbol(
        "kata_rt_broadcast_receiver_create",
        rt::kata_rt_broadcast_receiver_create as *const u8,
    );
    builder.symbol(
        "kata_rt_channel_send",
        rt::kata_rt_channel_send as *const u8,
    );
    builder.symbol(
        "kata_rt_channel_recv",
        rt::kata_rt_channel_recv as *const u8,
    );
    builder.symbol("kata_rt_select", rt::kata_rt_select as *const u8);
    // Log
    builder.symbol("kata_rt_log_publish", rt::kata_rt_log_publish as *const u8);
    builder.symbol("kata_rt_log_recv", rt::kata_rt_log_recv as *const u8);
    builder.symbol("kata_rt_log_config", rt::kata_rt_log_config as *const u8);
    // Comptime snapshots (Fio 12)
    builder.symbol(
        "kata_rt_load_snapshot",
        rt::kata_rt_load_snapshot as *const u8,
    );
    builder.symbol(
        "kata_rt_get_snapshot",
        rt::kata_rt_get_snapshot as *const u8,
    );
    // Cache @cache{strategy: "LRU"} (Fio 12, Fase 5)
    builder.symbol(
        "kata_rt_cache_get_or_create",
        rt::kata_rt_cache_get_or_create as *const u8,
    );
    builder.symbol(
        "kata_rt_cache_lookup",
        rt::kata_rt_cache_lookup as *const u8,
    );
    builder.symbol(
        "kata_rt_cache_insert",
        rt::kata_rt_cache_insert as *const u8,
    );
    builder.symbol(
        "kata_rt_serialize_key",
        rt::kata_rt_serialize_key as *const u8,
    );
    // Bytes / Byte (PRD-bytes)
    builder.symbol("kata_rt_bytes_alloc", rt::kata_rt_bytes_alloc as *const u8);
    builder.symbol(
        "kata_rt_bytes_from_ptr",
        rt::kata_rt_bytes_from_ptr as *const u8,
    );
    builder.symbol(
        "kata_rt_bytes_from_ints",
        rt::kata_rt_bytes_from_ints as *const u8,
    );
    builder.symbol("kata_rt_bytes_len", rt::kata_rt_bytes_len as *const u8);
    builder.symbol("kata_rt_bytes_get", rt::kata_rt_bytes_get as *const u8);
    builder.symbol("kata_rt_bytes_set", rt::kata_rt_bytes_set as *const u8);
    builder.symbol(
        "kata_rt_bytes_get_checked",
        rt::kata_rt_bytes_get_checked as *const u8,
    );
    builder.symbol(
        "kata_rt_bytes_concat",
        rt::kata_rt_bytes_concat as *const u8,
    );
    builder.symbol("kata_rt_bytes_eq", rt::kata_rt_bytes_eq as *const u8);
    builder.symbol("kata_rt_bytes_neq", rt::kata_rt_bytes_neq as *const u8);
    builder.symbol("kata_rt_bytes_show", rt::kata_rt_bytes_show as *const u8);
    builder.symbol("kata_rt_bytes_slice", rt::kata_rt_bytes_slice as *const u8);
    builder.symbol("kata_rt_bytes_and", rt::kata_rt_bytes_and as *const u8);
    builder.symbol("kata_rt_bytes_or", rt::kata_rt_bytes_or as *const u8);
    builder.symbol("kata_rt_bytes_xor", rt::kata_rt_bytes_xor as *const u8);
    builder.symbol("kata_rt_bytes_not", rt::kata_rt_bytes_not as *const u8);
    builder.symbol("kata_rt_byte_and", rt::kata_rt_byte_and as *const u8);
    builder.symbol("kata_rt_byte_or", rt::kata_rt_byte_or as *const u8);
    builder.symbol("kata_rt_byte_xor", rt::kata_rt_byte_xor as *const u8);
    builder.symbol("kata_rt_byte_not", rt::kata_rt_byte_not as *const u8);
    builder.symbol("kata_rt_byte_shr", rt::kata_rt_byte_shr as *const u8);
    builder.symbol("kata_rt_byte_shl", rt::kata_rt_byte_shl as *const u8);
    builder.symbol("kata_rt_byte_to_int", rt::kata_rt_byte_to_int as *const u8);
    builder.symbol("kata_rt_int_to_byte", rt::kata_rt_int_to_byte as *const u8);
    builder.symbol(
        "kata_rt_int_to_bytes",
        rt::kata_rt_int_to_bytes as *const u8,
    );
    builder.symbol(
        "kata_rt_text_to_bytes",
        rt::kata_rt_text_to_bytes as *const u8,
    );
    builder.symbol(
        "kata_rt_bytes_to_text",
        rt::kata_rt_bytes_to_text as *const u8,
    );
    builder.symbol("kata_rt_text_at", rt::kata_rt_text_at as *const u8);
    builder.symbol("kata_rt_text_len", rt::kata_rt_text_len as *const u8);
    builder.symbol("kata_rt_text_slice", rt::kata_rt_text_slice as *const u8);
    builder.symbol("kata_rt_array_slice", rt::kata_rt_array_slice as *const u8);
    builder.symbol("kata_rt_list_slice", rt::kata_rt_list_slice as *const u8);
}

/// Declara todos os símbolos FFI no module e retorna o mapa nome → FuncId.
pub(crate) fn declare_ffi_symbols(
    module: &mut dyn ModuleBackend,
) -> Result<HashMap<String, cranelift_module::FuncId>, CodegenError> {
    let mut ffi_ids = HashMap::new();
    for sym in all_ffi_symbols() {
        let name = sym.symbol_name();
        let sig = ffi_signature(sym);
        let fid = module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| CodegenError::Cranelift(format!("declare FFI {name}: {e}")))?;
        ffi_ids.insert(name.to_string(), fid);
    }
    // Símbolo especial: kata_rt_tag_int_from_str (não está no FfiSymbol enum).
    // Usado para lowerar IntLit que não cabe em SMI (BigInts).
    let tag_str_sig = {
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(I64)); // ptr
        sig.params.push(AbiParam::new(I64)); // len
        sig.returns.push(AbiParam::new(I64)); // tagged i64
        sig
    };
    let tag_str_fid = module
        .declare_function("kata_rt_tag_int_from_str", Linkage::Import, &tag_str_sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare kata_rt_tag_int_from_str: {e}")))?;
    ffi_ids.insert("kata_rt_tag_int_from_str".to_string(), tag_str_fid);
    Ok(ffi_ids)
}

/// Todos os símbolos FFI que o codegen conhece.
fn all_ffi_symbols() -> Vec<FfiSymbol> {
    use FfiSymbol::*;
    vec![
        BiAdd,
        BiSub,
        BiMul,
        BiDiv,
        BiEq,
        BiNeq,
        BiLt,
        BiLe,
        BiGt,
        BiGe,
        BiShow,
        BiToRational,
        TagInt,
        IntToText,
        Fadd,
        Fsub,
        Fmul,
        Fdiv,
        FcmpEq,
        FcmpNeq,
        FcmpLt,
        FcmpLe,
        FcmpGt,
        FcmpGe,
        FloatToText,
        RatAdd,
        RatSub,
        RatMul,
        RatDiv,
        RatEq,
        RatNeq,
        RatLt,
        RatLe,
        RatGt,
        RatGe,
        RatShow,
        RatToFloat,
        RatFromFloat,
        RatLiteral,
        IntToRational,
        StringConcat,
        StringLen,
        TextLiteral,
        BoolToText,
        TextReplaceFirst,
        Print,
        Println,
        ArenaCreate,
        ArenaAlloc,
        ArenaDestroy,
        ArenaCreateTracked,
        ArenaDealloc,
        GetRootArenaHandle,
        AllocTracked,
        IncRefTracked,
        DecRefTracked,
        ArenaStats,
        StoreSumResult,
        SumTagInt,
        Panic,
        SchedulerInit,
        Spawn,
        Run,
        Yield,
        YieldCheck,
        SetTestTimeout,
        Sleep,
        AllocArc,
        IncRef,
        DecRef,
        ArcFnPtr,
        // Collections
        ListNil,
        ListCons,
        ListIsEmpty,
        ListHead,
        ListTail,
        ListLen,
        ListGetChecked,
        ArrayAlloc,
        ArrayLen,
        ArrayGet,
        ArraySet,
        ArrayGetChecked,
        RangeAlloc,
        ListContains,
        ArrayContains,
        ListReverse,
        ListConcat,
        // Hash (Fio 13)
        HashInt,
        HashText,
        HashRational,
        // Dict (Fio 13)
        DictEmpty,
        DictInsert,
        DictGetChecked,
        DictContains,
        DictLen,
        DictRemove,
        DictNext,
        // Set (Fio 13)
        SetEmpty,
        SetInsert,
        SetContains,
        SetLen,
        SetRemove,
        SetNext,
        SetUnion,
        SetIntersection,
        SetDifference,
        DictMerge,
        // String equality (Fio 13)
        StringEq,
        // Canais CSP
        ChannelCreate,
        QueueCreate,
        BroadcastCreate,
        BroadcastReceiverCreate,
        ChannelSend,
        ChannelRecv,
        ChannelSelect,
        // Log
        LogPublish,
        LogRecv,
        LogConfig,
        // Comptime snapshots (Fio 12)
        LoadSnapshot,
        GetSnapshot,
        // Cache @cache{strategy: "LRU"} (Fio 12, Fase 5)
        CacheGetOrCreate,
        CacheLookup,
        CacheInsert,
        CacheSerializeKey,
        // Bytes / Byte (PRD-bytes)
        BytesAlloc,
        BytesFromPtr,
        BytesFromInts,
        BytesLen,
        BytesGet,
        BytesSet,
        BytesGetChecked,
        BytesConcat,
        BytesEq,
        BytesNeq,
        BytesShow,
        BytesSlice,
        BytesAnd,
        BytesOr,
        BytesXor,
        BytesNot,
        ByteAnd,
        ByteOr,
        ByteXor,
        ByteNot,
        ByteShr,
        ByteShl,
        ByteToInt,
        IntToByte,
        IntToBytes,
        TextToBytes,
        BytesToText,
        TextAt,
        TextLen,
        TextSlice,
        ArraySlice,
        ListSlice,
    ]
}
