//! Runtime isolado da Kata-Lang.
//!
//! BigInt/SMI tagging, Float, Rational, Text, arena, print.
//! Linkada via symbol map (C-ABI). Desconhece as regras internas da linguagem.
//!
//! O compilador conhece apenas o enum `FfiSymbol` e as 3 strings de mapeamento
//! (`"i64"`, `"f64"`, `"kata_rt_string"`). Toda a implementação vive aqui.

pub(crate) mod arc;
pub(crate) mod arena;
pub(crate) mod array;
pub(crate) mod bigint;
pub(crate) mod byte;
pub(crate) mod bytes;
pub(crate) mod cache;
pub(crate) mod channel;
pub(crate) mod convert;
pub(crate) mod dict;
pub(crate) mod display;
pub(crate) mod fiber;
pub(crate) mod float;
pub(crate) mod hash;
pub(crate) mod io;
pub(crate) mod list;
pub(crate) mod log;
pub(crate) mod range;
pub(crate) mod rational;
pub(crate) mod scheduler;
pub(crate) mod set;
pub(crate) mod slice;
pub(crate) mod snapshot;
pub(crate) mod sum;
pub(crate) mod text;
pub(crate) mod tracked;

// Re-exports convenientes para uso interno (não C-ABI).
// Símbolos não consumidos cross-crate foram removidos (to_rational, float_to_rat,
// rat_from_int, rat_to_float, rat_to_string, reset_all_arenas, reset_scheduler).
pub use bigint::{
    bigint_to_string, decode_smi_pub, encode_smi_pub, fits_smi_pub, is_smi_pub, show,
    tag_int_from_str, tag_int_pub,
};
pub use float::float_to_string;
pub use rational::rat_from_text;
// text::{bool_to_text, int_to_text, text_literal, text_replace_first} —
// rebaixados para pub(crate): zero consumidores cross-crate (apenas wrappers
// C-ABI kata_rt_* no próprio text.rs os usam internamente).

// Re-exports de funções C-ABI para o codegen registrar no JIT.
pub use arc::{kata_rt_alloc_arc, kata_rt_arc_fn_ptr, kata_rt_decref, kata_rt_incref};
pub use arena::{
    kata_rt_arena_alloc, kata_rt_arena_create, kata_rt_arena_create_tracked, kata_rt_arena_dealloc,
    kata_rt_arena_destroy, kata_rt_get_root_arena_handle,
};
pub use array::{
    kata_rt_array_alloc, kata_rt_array_contains, kata_rt_array_get, kata_rt_array_get_checked,
    kata_rt_array_len, kata_rt_array_set,
};
pub use bigint::{
    kata_rt_bi_add, kata_rt_bi_div, kata_rt_bi_eq, kata_rt_bi_ge, kata_rt_bi_gt, kata_rt_bi_le,
    kata_rt_bi_lt, kata_rt_bi_mul, kata_rt_bi_neq, kata_rt_bi_show, kata_rt_bi_sub,
    kata_rt_bi_to_rational, kata_rt_int_to_text, kata_rt_tag_int, kata_rt_tag_int_from_str,
};
pub use byte::{
    kata_rt_byte_and, kata_rt_byte_not, kata_rt_byte_or, kata_rt_byte_shl, kata_rt_byte_shr,
    kata_rt_byte_to_int, kata_rt_byte_xor, kata_rt_int_to_byte,
};
pub use bytes::{
    kata_rt_bytes_alloc, kata_rt_bytes_and, kata_rt_bytes_concat, kata_rt_bytes_eq,
    kata_rt_bytes_from_ints, kata_rt_bytes_from_ptr, kata_rt_bytes_get, kata_rt_bytes_get_checked,
    kata_rt_bytes_len, kata_rt_bytes_neq, kata_rt_bytes_not, kata_rt_bytes_or, kata_rt_bytes_set,
    kata_rt_bytes_show, kata_rt_bytes_slice, kata_rt_bytes_xor,
};
pub use convert::{kata_rt_bytes_to_text, kata_rt_int_to_bytes, kata_rt_text_to_bytes};
pub use dict::kata_rt_dict_merge;
pub use dict::{
    kata_rt_dict_contains, kata_rt_dict_empty, kata_rt_dict_get_checked, kata_rt_dict_insert,
    kata_rt_dict_len, kata_rt_dict_next, kata_rt_dict_remove,
};
pub use float::{
    kata_rt_fadd, kata_rt_fcmp_eq, kata_rt_fcmp_ge, kata_rt_fcmp_gt, kata_rt_fcmp_le,
    kata_rt_fcmp_lt, kata_rt_fcmp_neq, kata_rt_fdiv, kata_rt_float_to_text, kata_rt_fmul,
    kata_rt_fsub,
};
pub use io::{kata_rt_panic, kata_rt_print, kata_rt_println};
pub use list::{
    kata_rt_list_concat, kata_rt_list_cons, kata_rt_list_contains, kata_rt_list_get_checked,
    kata_rt_list_head, kata_rt_list_is_empty, kata_rt_list_len, kata_rt_list_nil,
    kata_rt_list_reverse, kata_rt_list_tail,
};
pub use range::kata_rt_range_alloc;
pub use rational::{
    kata_rt_int_to_rational, kata_rt_rat_add, kata_rt_rat_div, kata_rt_rat_eq,
    kata_rt_rat_from_float, kata_rt_rat_ge, kata_rt_rat_gt, kata_rt_rat_le, kata_rt_rat_literal,
    kata_rt_rat_lt, kata_rt_rat_mul, kata_rt_rat_neq, kata_rt_rat_show, kata_rt_rat_sub,
    kata_rt_rat_to_float,
};
pub use set::{
    kata_rt_set_contains, kata_rt_set_difference, kata_rt_set_empty, kata_rt_set_insert,
    kata_rt_set_intersection, kata_rt_set_len, kata_rt_set_next, kata_rt_set_remove,
    kata_rt_set_union,
};
pub use slice::{
    kata_rt_array_slice, kata_rt_list_slice, kata_rt_text_at, kata_rt_text_len, kata_rt_text_slice,
};
pub use tracked::{kata_rt_alloc_tracked, kata_rt_decref_tracked, kata_rt_incref_tracked};
// kata_rt_rat_show_raw — rebaixado para pub(crate): zero consumidores cross-crate.
pub use scheduler::{
    DEADLOCK_SENTINEL, TIMEOUT_SENTINEL, kata_rt_run, kata_rt_scheduler_init,
    kata_rt_set_test_timeout, kata_rt_sleep, kata_rt_spawn, kata_rt_yield, kata_rt_yield_check,
    reset_scheduler,
};
pub use sum::{kata_rt_store_sum_result, kata_rt_sum_tag_int};
// Snapshots comptime — carregados em load-time na root_arena
pub use snapshot::{kata_rt_get_snapshot, kata_rt_load_snapshot};
// Cache @cache{strategy: "LRU"}
pub use cache::{
    kata_rt_cache_get_or_create, kata_rt_cache_insert, kata_rt_cache_lookup, kata_rt_serialize_key,
};
pub use text::{
    kata_rt_bool_to_text, kata_rt_string_concat, kata_rt_string_eq, kata_rt_string_len,
    kata_rt_text_literal, kata_rt_text_replace_first,
};
// Canais CSP
pub use channel::{
    kata_rt_broadcast_create, kata_rt_broadcast_receiver_create, kata_rt_channel_create,
    kata_rt_channel_recv, kata_rt_channel_send, kata_rt_queue_create, kata_rt_select,
};
// Telemetria (@log)
// reset_log e snapshot_log_config — rebaixados para pub(crate): zero
// consumidores cross-crate (apenas scheduler.rs intra-crate os chama).
pub use log::{kata_rt_log_config, kata_rt_log_publish, kata_rt_log_recv};
// Display de resultados — ponto único de display para
// driver JIT e shim AOT.
pub use display::{
    TYPE_BOOLEAN, TYPE_FLOAT, TYPE_INT, TYPE_OTHER, TYPE_RATIONAL, TYPE_TEXT, TYPE_UNIT,
    kata_rt_print_result,
};
// Hash — FNV-1a para Int, Text, Rational
pub use hash::{kata_rt_hash_int, kata_rt_hash_rational, kata_rt_hash_text};
