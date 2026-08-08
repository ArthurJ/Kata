//! `FfiSymbol` — enum tipado de símbolos FFI.
//!
//! Substitui strings soltas — se você errar o símbolo, é erro de compilação
//! do compilador, não bug silencioso em runtime. Cada variante carrega
//! metadados: `symbol_name()`, `return_type()`, etc.

use crate::ty::Ty;

/// Símbolo FFI catalogado. O compilador conhece apenas isto e as 3 strings
/// de mapeamento de representação (`"i64"`, `"f64"`, `"kata_rt_string"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FfiSymbol {
    // ── BigInt (Int com SMI tagging) ─────────────────────
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

    // ── Float ────────────────────────────────────────────
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

    // ── Rational ─────────────────────────────────────────
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

    // ── Text ─────────────────────────────────────────────
    StringConcat,
    StringLen,
    TextLiteral,
    IntToText,
    BoolToText,
    TextReplaceFirst,

    // ── I/O ──────────────────────────────────────────────
    Print,
    Println,

    // ── Arena ────────────────────────────────────────────
    ArenaCreate,
    ArenaAlloc,
    ArenaDestroy,
    /// `kata_rt_arena_create_tracked() -> handle` — cria arena Tracked (Fio 16).
    ArenaCreateTracked,
    /// `kata_rt_arena_dealloc(handle, ptr, size) -> void` — dealloc individual (Fio 16).
    ArenaDealloc,
    /// `kata_rt_get_root_arena_handle() -> handle` — lê TLS root arena (Fio 16).
    GetRootArenaHandle,
    /// `kata_rt_arena_stats(handle) -> i64` — (alloc_count, dealloc_count) packed (Fio 16).
    ArenaStats,

    // ── Sum ──────────────────────────────────────
    /// `kata_rt_store_sum_result(tag, payload) -> ptr` — aloca box Sum.
    StoreSumResult,
    /// `kata_rt_sum_tag_int(val) -> tag` — extrai tag de Sum box.
    SumTagInt,

    // ── Control flow ───────────────────────────────
    /// `kata_rt_panic(msg) -> !` — aborta com mensagem.
    Panic,

    // ── Scheduler/Fiber ───────────────────────────
    /// `kata_rt_scheduler_init() -> i64` — inicializa scheduler thread-local.
    SchedulerInit,
    /// `kata_rt_spawn(fn_ptr, caller_arena, args_ptr) -> i64` — cria fiber.
    Spawn,
    /// `kata_rt_run() -> i64` — executa próximo fiber, retorna resultado.
    Run,
    /// `kata_rt_yield() -> ()` — suspende fiber atual (não usado).
    Yield,
    /// `kata_rt_yield_check() -> ()` — yield point no header de loops.
    YieldCheck,
    /// `kata_rt_set_test_timeout(millis) -> ()` — configura timeout de teste.
    /// Spawna thread OS timer que seta `TIMEOUT_EXPIRED` ao expirar.
    SetTestTimeout,
    /// `kata_rt_sleep(ms) -> ()` — sleep cooperativo (suspende fiber até deadline).
    Sleep,

    // ── Arc<T> / CaptureBox ───────────────────────
    /// `kata_rt_alloc_arc(fn_ptr, captures_ptr, n_captures) -> box_ptr`
    AllocArc,
    /// `kata_rt_incref(box_ptr) -> 0` — incrementa refcount.
    IncRef,
    /// `kata_rt_decref(box_ptr) -> 0` — decrementa refcount.
    DecRef,
    /// `kata_rt_arc_fn_ptr(box_ptr) -> fn_ptr` — extrai fn_ptr do box.
    ArcFnPtr,

    // ── Collections ───────────────────────────
    /// `kata_rt_list_nil() -> ptr` — retorna 0 (null = Nil).
    ListNil,
    /// `kata_rt_list_cons(head, tail, arena) -> ptr` — aloca Cons cell.
    ListCons,
    /// `kata_rt_list_is_empty(ptr) -> i64` — ptr == 0?
    ListIsEmpty,
    /// `kata_rt_list_head(ptr) -> i64` — load ptr+0.
    ListHead,
    /// `kata_rt_list_tail(ptr) -> ptr` — load ptr+8.
    ListTail,
    /// `kata_rt_list_len(ptr) -> i64` — conta Cons cells.
    ListLen,
    /// `kata_rt_list_get_checked(ptr, idx) -> ptr` — Result box.
    ListGetChecked,
    /// `kata_rt_array_alloc(len, arena) -> ptr` — aloca header + data.
    ArrayAlloc,
    /// `kata_rt_array_len(ptr) -> i64` — load ptr+0.
    ArrayLen,
    /// `kata_rt_array_get(ptr, idx) -> i64` — load ptr+8+idx*8.
    ArrayGet,
    /// `kata_rt_array_set(ptr, idx, val) -> ()` — store ptr+8+idx*8.
    ArraySet,
    /// `kata_rt_array_get_checked(ptr, idx) -> ptr` — Result box.
    ArrayGetChecked,
    /// `kata_rt_range_alloc(arena) -> ptr` — aloca 3 words.
    RangeAlloc,
    /// `kata_rt_list_contains(ptr, item) -> i64` — percorre Cons cells.
    ListContains,
    /// `kata_rt_array_contains(ptr, item) -> i64` — percorre array.
    ArrayContains,
    /// `kata_rt_list_reverse(ptr, arena) -> ptr` — inverte Cons chain.
    ListReverse,
    /// `kata_rt_list_concat(first, second, arena) -> ptr` — concatena duas listas.
    ListConcat,

    // ── Hash (Fio 13) ───────────────────────────────────
    /// `kata_rt_hash_int(val) -> i64` — FNV-1a hash de Int (SMI-tagged).
    HashInt,
    /// `kata_rt_hash_text(str_ptr) -> i64` — FNV-1a hash de Text.
    HashText,
    /// `kata_rt_hash_rational(rat_ptr) -> i64` — FNV-1a hash de Rational.
    HashRational,

    // ── Dict (Fio 13) ───────────────────────────────────
    /// `kata_rt_dict_empty(arena) -> i64` — aloca Dict vazio.
    DictEmpty,
    /// `kata_rt_dict_insert(dict, key, val, hash, eq_fn, arena) -> i64` — insere par.
    DictInsert,
    /// `kata_rt_dict_get_checked(dict, key, hash, eq_fn, arena) -> i64` — Result box.
    DictGetChecked,
    /// `kata_rt_dict_contains(dict, key, hash, eq_fn) -> i64` — 0/1.
    DictContains,
    /// `kata_rt_dict_len(dict) -> i64` — contagem.
    DictLen,
    /// `kata_rt_dict_remove(dict, key, hash, eq_fn, arena) -> i64` — novo Dict.
    DictRemove,
    /// `kata_rt_dict_next(dict, iter_state, arena) -> i64` — Optional box.
    DictNext,

    // ── Set (Fio 13) ────────────────────────────────────
    /// `kata_rt_set_empty(arena) -> i64` — delega para dict_empty.
    SetEmpty,
    /// `kata_rt_set_insert(set, elem, hash, eq_fn, arena) -> i64` — novo Set.
    SetInsert,
    /// `kata_rt_set_contains(set, elem, hash, eq_fn) -> i64` — 0/1.
    SetContains,
    /// `kata_rt_set_len(set) -> i64` — contagem.
    SetLen,
    /// `kata_rt_set_remove(set, elem, hash, eq_fn, arena) -> i64` — novo Set.
    SetRemove,
    /// `kata_rt_set_next(set, iter_state, arena) -> i64` — Optional box.
    SetNext,
    /// `kata_rt_set_union(a, b, eq_fn, arena) -> i64` — união.
    SetUnion,
    /// `kata_rt_set_intersection(a, b, eq_fn, arena) -> i64` — intersecção.
    SetIntersection,
    /// `kata_rt_set_difference(a, b, eq_fn, arena) -> i64` — diferença.
    SetDifference,
    /// `kata_rt_dict_merge(a, b, eq_fn, arena) -> i64` — merge right-biased.
    DictMerge,

    // ── String equality (for Text keys in Dict/Set) ─────
    /// `kata_rt_string_eq(a, b) -> i64` — compara C strings por conteúdo.
    StringEq,

    // ── Canais CSP ────────────────────────────
    /// `kata_rt_channel_create(arena) -> i64` — canal rendezvous.
    ChannelCreate,
    /// `kata_rt_queue_create(arena, capacity) -> i64` — fila bufferizada.
    QueueCreate,
    /// `kata_rt_broadcast_create(arena) -> i64` — broadcast pub-sub.
    BroadcastCreate,
    /// `kata_rt_broadcast_receiver_create(arena, factory) -> i64` — receiver.
    BroadcastReceiverCreate,
    /// `kata_rt_ipc_channel_create(arena, type_id, ack_tx_handle) -> i64` — canal cross-process (pipe).
    IpcChannelCreate,
    /// `kata_rt_ipc_queue_create(arena, cap, type_id) -> ptr` — queue IPC cross-process.
    /// Cria in-process queue + IPC data channel + IPC ack channel. Retorna
    /// ponteiro para tupla de 6 handles na arena.
    IpcQueueCreate,
    /// `kata_rt_channel_send(handle, value) -> i64` — envia (0=OK, -1=block).
    ChannelSend,
    /// `kata_rt_channel_recv(handle) -> i64` — recebe (valor ou -1=block).
    ChannelRecv,
    /// `kata_rt_select(handles, n) -> i64` — select multiplex.
    ChannelSelect,
    /// `kata_rt_select_files(handles, n) -> i64` — select multiplex para file handles.
    SelectFiles,
    /// `kata_rt_select_combined(chan_ptr, n_c, file_ptr, n_f, timeout_ms) -> i64` —
    /// select combinado de channels e files com suspensão atômica.
    SelectCombined,
    /// `kata_rt_log_publish(topic_ptr, level, msg, policy_ptr) -> i64` — publica msg no tópico.
    LogPublish,
    /// `kata_rt_log_recv(topic_ptr) -> i64` — recebe próxima mensagem do tópico.
    LogRecv,
    /// `kata_rt_log_config(topic_ptr, policy_ptr, level) -> ()` — setta defaults de logging.
    LogConfig,

    // ── Comptime snapshots (Fio 12) ──────────────────────
    /// `kata_rt_load_snapshot(root_arena, bytes_ptr, bytes_len, rebase_offsets_ptr, rebase_count, snapshot_id) -> ()`
    /// — carrega um snapshot na root_arena e armazena na tabela TLS.
    LoadSnapshot,
    /// `kata_rt_get_snapshot(snapshot_id) -> ptr` — retorna ponteiro do snapshot da tabela TLS.
    GetSnapshot,

    // ── Cache @cache{strategy: "LRU"} (Fio 12, Fase 5) ──
    /// `kata_rt_cache_get_or_create(arena, fn_id, capacity) -> handle`
    CacheGetOrCreate,
    /// `kata_rt_cache_lookup(handle, key_ptr, key_len) -> i64` (0=miss, ptr=hit)
    CacheLookup,
    /// `kata_rt_cache_insert(handle, key_ptr, key_len, value) -> ()`
    CacheInsert,
    /// `kata_rt_serialize_key(value, desc_ptr, desc_len, out_ptr, out_cap) -> i64`
    CacheSerializeKey,

    // ── Bytes / Byte (PRD-bytes) ────────────────────────
    /// `kata_rt_bytes_alloc(len, arena) -> ptr`
    BytesAlloc,
    /// `kata_rt_bytes_from_ptr(src, len, arena) -> ptr`
    BytesFromPtr,
    /// `kata_rt_bytes_from_ints(ptrs, count, arena) -> ptr`
    BytesFromInts,
    /// `kata_rt_bytes_len(ptr) -> i64`
    BytesLen,
    /// `kata_rt_bytes_get(ptr, idx) -> i64`
    BytesGet,
    /// `kata_rt_bytes_set(ptr, idx, val) -> void`
    BytesSet,
    /// `kata_rt_bytes_get_checked(ptr, idx) -> i64 (Result box)`
    BytesGetChecked,
    /// `kata_rt_bytes_concat(a, b, arena) -> ptr`
    BytesConcat,
    /// `kata_rt_bytes_eq(a, b) -> i64 (0/1)`
    BytesEq,
    /// `kata_rt_bytes_neq(a, b) -> i64 (0/1)`
    BytesNeq,
    /// `kata_rt_bytes_show(ptr) -> *mut c_char`
    BytesShow,
    /// `kata_rt_bytes_slice(ptr, start, end, arena) -> ptr`
    BytesSlice,
    /// `kata_rt_bytes_and(a, b, arena) -> ptr`
    BytesAnd,
    /// `kata_rt_bytes_or(a, b, arena) -> ptr`
    BytesOr,
    /// `kata_rt_bytes_xor(a, b, arena) -> ptr`
    BytesXor,
    /// `kata_rt_bytes_not(ptr, arena) -> ptr`
    BytesNot,
    /// `kata_rt_byte_and(a, b) -> i64`
    ByteAnd,
    /// `kata_rt_byte_or(a, b) -> i64`
    ByteOr,
    /// `kata_rt_byte_xor(a, b) -> i64`
    ByteXor,
    /// `kata_rt_byte_not(a) -> i64`
    ByteNot,
    /// `kata_rt_byte_shr(a, n) -> i64`
    ByteShr,
    /// `kata_rt_byte_shl(a, n) -> i64`
    ByteShl,
    /// `kata_rt_byte_to_int(b) -> i64`
    ByteToInt,
    /// `kata_rt_int_to_byte(n) -> i64`
    IntToByte,
    /// `kata_rt_int_to_bytes(n, arena) -> ptr`
    IntToBytes,
    /// `kata_rt_text_to_bytes(text_ptr, arena) -> ptr`
    TextToBytes,
    /// `kata_rt_bytes_to_text(bytes_ptr) -> *mut c_char`
    BytesToText,
    /// `kata_rt_text_at(text_ptr, idx, arena) -> i64 (Result box)`
    TextAt,
    /// `kata_rt_text_len(text_ptr) -> i64`
    TextLen,
    /// `kata_rt_text_slice(text_ptr, start, end) -> *mut c_char`
    TextSlice,
    /// `kata_rt_array_slice(ptr, start, end, arena) -> ptr`
    ArraySlice,
    /// `kata_rt_list_slice(ptr, start, end, arena) -> ptr`
    ListSlice,
    /// `kata_rt_to_bytes(value_ptr, type_id, arena) -> bytes_ptr`
    ToBytes,
    /// `kata_rt_from_bytes(bytes_ptr, arena) -> value_ptr`
    FromBytes,
    /// `kata_rt_spawn_process(fn_ptr, args_ptr, result_type_id, arena) -> value_ptr`
    /// Spawn um processo OS separado via fork+pipe. O child herda a
    /// arena via COW, executa a Action, serializa o resultado via
    /// to_bytes, e envia pelo pipe. O parent desserializa via from_bytes.
    SpawnProcess,

    // ── File I/O ─────────────────────────────────────────
    /// `kata_rt_file_open(path_ptr, mode_tag) -> i64` — abre arquivo, retorna Result box ARC.
    FileOpen,
    /// `kata_rt_file_read(handle) -> i64` — lê todo o conteúdo, retorna Result box ARC.
    FileRead,
    /// `kata_rt_file_read_chunk(handle, n) -> i64` — lê até n bytes, retorna Result box.
    FileReadChunk,
    /// `kata_rt_file_readline(handle) -> i64` — lê uma linha, retorna Result box ARC.
    FileReadline,
    /// `kata_rt_file_write_text(handle, data_ptr) -> i64` — escreve Text (C string), retorna Result box ARC.
    FileWriteText,
    /// `kata_rt_file_write_bytes(handle, data_ptr) -> i64` — escreve Bytes (blob com header de len), retorna Result box ARC.
    FileWriteBytes,
    /// `kata_rt_file_close(handle) -> ()` — fecha arquivo (decref ARC).
    FileClose,

    // ── Socket I/O ─────────────────────────────────────────────────
    /// `kata_rt_socket_open(kind_box, mode_box) -> i64` — cria socket, retorna Result box.
    SocketOpen,
    /// `kata_rt_socket_listen(listener_handle) -> i64` — accept, retorna Result box.
    SocketListen,
    /// `kata_rt_socket_read(handle) -> i64` — lê todo o disponível, retorna Result box.
    SocketRead,
    /// `kata_rt_socket_read_chunk(handle, n) -> i64` — lê até n bytes, retorna Result box.
    SocketReadChunk,
    /// `kata_rt_socket_readline(handle) -> i64` — lê uma linha (Text), retorna Result box.
    SocketReadline,
    /// `kata_rt_socket_write_text(handle, data_ptr) -> i64` — escreve Text, retorna Result box.
    SocketWriteText,
    /// `kata_rt_socket_write_bytes(handle, data_ptr) -> i64` — escreve Bytes, retorna Result box.
    SocketWriteBytes,
    /// `kata_rt_socket_close(handle) -> ()` — fecha socket (idempotente).
    SocketClose,
    /// `kata_rt_timer_now() -> i64` — clock monotônico em nanossegundos.
    TimerNow,
}

impl FfiSymbol {
    /// Nome do símbolo C no runtime.
    pub fn symbol_name(self) -> &'static str {
        match self {
            FfiSymbol::BiAdd => "kata_rt_bi_add",
            FfiSymbol::BiSub => "kata_rt_bi_sub",
            FfiSymbol::BiMul => "kata_rt_bi_mul",
            FfiSymbol::BiDiv => "kata_rt_bi_div",
            FfiSymbol::BiEq => "kata_rt_bi_eq",
            FfiSymbol::BiNeq => "kata_rt_bi_neq",
            FfiSymbol::BiLt => "kata_rt_bi_lt",
            FfiSymbol::BiLe => "kata_rt_bi_le",
            FfiSymbol::BiGt => "kata_rt_bi_gt",
            FfiSymbol::BiGe => "kata_rt_bi_ge",
            FfiSymbol::BiShow => "kata_rt_bi_show",
            FfiSymbol::BiToRational => "kata_rt_bi_to_rational",
            FfiSymbol::TagInt => "kata_rt_tag_int",
            FfiSymbol::Fadd => "kata_rt_fadd",
            FfiSymbol::Fsub => "kata_rt_fsub",
            FfiSymbol::Fmul => "kata_rt_fmul",
            FfiSymbol::Fdiv => "kata_rt_fdiv",
            FfiSymbol::FcmpEq => "kata_rt_fcmp_eq",
            FfiSymbol::FcmpNeq => "kata_rt_fcmp_neq",
            FfiSymbol::FcmpLt => "kata_rt_fcmp_lt",
            FfiSymbol::FcmpLe => "kata_rt_fcmp_le",
            FfiSymbol::FcmpGt => "kata_rt_fcmp_gt",
            FfiSymbol::FcmpGe => "kata_rt_fcmp_ge",
            FfiSymbol::FloatToText => "kata_rt_float_to_text",
            FfiSymbol::RatAdd => "kata_rt_rat_add",
            FfiSymbol::RatSub => "kata_rt_rat_sub",
            FfiSymbol::RatMul => "kata_rt_rat_mul",
            FfiSymbol::RatDiv => "kata_rt_rat_div",
            FfiSymbol::RatEq => "kata_rt_rat_eq",
            FfiSymbol::RatNeq => "kata_rt_rat_neq",
            FfiSymbol::RatLt => "kata_rt_rat_lt",
            FfiSymbol::RatLe => "kata_rt_rat_le",
            FfiSymbol::RatGt => "kata_rt_rat_gt",
            FfiSymbol::RatGe => "kata_rt_rat_ge",
            FfiSymbol::RatShow => "kata_rt_rat_show",
            FfiSymbol::RatToFloat => "kata_rt_rat_to_float",
            FfiSymbol::RatFromFloat => "kata_rt_rat_from_float",
            FfiSymbol::RatLiteral => "kata_rt_rat_literal",
            FfiSymbol::IntToRational => "kata_rt_int_to_rational",
            FfiSymbol::StringConcat => "kata_rt_string_concat",
            FfiSymbol::StringLen => "kata_rt_string_len",
            FfiSymbol::TextLiteral => "kata_rt_text_literal",
            FfiSymbol::IntToText => "kata_rt_int_to_text",
            FfiSymbol::BoolToText => "kata_rt_bool_to_text",
            FfiSymbol::TextReplaceFirst => "kata_rt_text_replace_first",
            FfiSymbol::Print => "kata_rt_print",
            FfiSymbol::Println => "kata_rt_println",
            FfiSymbol::ArenaCreate => "kata_rt_arena_create",
            FfiSymbol::ArenaAlloc => "kata_rt_arena_alloc",
            FfiSymbol::ArenaDestroy => "kata_rt_arena_destroy",
            FfiSymbol::ArenaCreateTracked => "kata_rt_arena_create_tracked",
            FfiSymbol::ArenaDealloc => "kata_rt_arena_dealloc",
            FfiSymbol::GetRootArenaHandle => "kata_rt_get_root_arena_handle",
            FfiSymbol::ArenaStats => "kata_rt_arena_stats",
            FfiSymbol::StoreSumResult => "kata_rt_store_sum_result",
            FfiSymbol::SumTagInt => "kata_rt_sum_tag_int",
            FfiSymbol::Panic => "kata_rt_panic",
            FfiSymbol::SchedulerInit => "kata_rt_scheduler_init",
            FfiSymbol::Spawn => "kata_rt_spawn",
            FfiSymbol::SpawnProcess => "kata_rt_spawn_process",
            FfiSymbol::Run => "kata_rt_run",
            FfiSymbol::Yield => "kata_rt_yield",
            FfiSymbol::YieldCheck => "kata_rt_yield_check",
            FfiSymbol::SetTestTimeout => "kata_rt_set_test_timeout",
            FfiSymbol::Sleep => "kata_rt_sleep",
            FfiSymbol::AllocArc => "kata_rt_alloc_arc",
            FfiSymbol::IncRef => "kata_rt_incref",
            FfiSymbol::DecRef => "kata_rt_decref",
            FfiSymbol::ArcFnPtr => "kata_rt_arc_fn_ptr",
            // Collections
            FfiSymbol::ListNil => "kata_rt_list_nil",
            FfiSymbol::ListCons => "kata_rt_list_cons",
            FfiSymbol::ListIsEmpty => "kata_rt_list_is_empty",
            FfiSymbol::ListHead => "kata_rt_list_head",
            FfiSymbol::ListTail => "kata_rt_list_tail",
            FfiSymbol::ListLen => "kata_rt_list_len",
            FfiSymbol::ListGetChecked => "kata_rt_list_get_checked",
            FfiSymbol::ArrayAlloc => "kata_rt_array_alloc",
            FfiSymbol::ArrayLen => "kata_rt_array_len",
            FfiSymbol::ArrayGet => "kata_rt_array_get",
            FfiSymbol::ArraySet => "kata_rt_array_set",
            FfiSymbol::ArrayGetChecked => "kata_rt_array_get_checked",
            FfiSymbol::RangeAlloc => "kata_rt_range_alloc",
            FfiSymbol::ListContains => "kata_rt_list_contains",
            FfiSymbol::ArrayContains => "kata_rt_array_contains",
            FfiSymbol::ListReverse => "kata_rt_list_reverse",
            FfiSymbol::ListConcat => "kata_rt_list_concat",
            // Hash (Fio 13)
            FfiSymbol::HashInt => "kata_rt_hash_int",
            FfiSymbol::HashText => "kata_rt_hash_text",
            FfiSymbol::HashRational => "kata_rt_hash_rational",
            // Dict (Fio 13)
            FfiSymbol::DictEmpty => "kata_rt_dict_empty",
            FfiSymbol::DictInsert => "kata_rt_dict_insert",
            FfiSymbol::DictGetChecked => "kata_rt_dict_get_checked",
            FfiSymbol::DictContains => "kata_rt_dict_contains",
            FfiSymbol::DictLen => "kata_rt_dict_len",
            FfiSymbol::DictRemove => "kata_rt_dict_remove",
            FfiSymbol::DictNext => "kata_rt_dict_next",
            // Set (Fio 13)
            FfiSymbol::SetEmpty => "kata_rt_set_empty",
            FfiSymbol::SetInsert => "kata_rt_set_insert",
            FfiSymbol::SetContains => "kata_rt_set_contains",
            FfiSymbol::SetLen => "kata_rt_set_len",
            FfiSymbol::SetRemove => "kata_rt_set_remove",
            FfiSymbol::SetNext => "kata_rt_set_next",
            FfiSymbol::SetUnion => "kata_rt_set_union",
            FfiSymbol::SetIntersection => "kata_rt_set_intersection",
            FfiSymbol::SetDifference => "kata_rt_set_difference",
            FfiSymbol::DictMerge => "kata_rt_dict_merge",
            // String equality (Fio 13)
            FfiSymbol::StringEq => "kata_rt_string_eq",
            // Canais CSP
            FfiSymbol::ChannelCreate => "kata_rt_channel_create",
            FfiSymbol::QueueCreate => "kata_rt_queue_create",
            FfiSymbol::BroadcastCreate => "kata_rt_broadcast_create",
            FfiSymbol::BroadcastReceiverCreate => "kata_rt_broadcast_receiver_create",
            FfiSymbol::IpcChannelCreate => "kata_rt_ipc_channel_create",
            FfiSymbol::IpcQueueCreate => "kata_rt_ipc_queue_create",
            FfiSymbol::ChannelSend => "kata_rt_channel_send",
            FfiSymbol::ChannelRecv => "kata_rt_channel_recv",
            FfiSymbol::ChannelSelect => "kata_rt_select",
            FfiSymbol::SelectFiles => "kata_rt_select_files",
            FfiSymbol::SelectCombined => "kata_rt_select_combined",
            // Log
            FfiSymbol::LogPublish => "kata_rt_log_publish",
            FfiSymbol::LogRecv => "kata_rt_log_recv",
            FfiSymbol::LogConfig => "kata_rt_log_config",
            // Comptime snapshots
            FfiSymbol::LoadSnapshot => "kata_rt_load_snapshot",
            FfiSymbol::GetSnapshot => "kata_rt_get_snapshot",
            // Cache
            FfiSymbol::CacheGetOrCreate => "kata_rt_cache_get_or_create",
            FfiSymbol::CacheLookup => "kata_rt_cache_lookup",
            FfiSymbol::CacheInsert => "kata_rt_cache_insert",
            FfiSymbol::CacheSerializeKey => "kata_rt_serialize_key",
            // Bytes / Byte (PRD-bytes)
            FfiSymbol::BytesAlloc => "kata_rt_bytes_alloc",
            FfiSymbol::BytesFromPtr => "kata_rt_bytes_from_ptr",
            FfiSymbol::BytesFromInts => "kata_rt_bytes_from_ints",
            FfiSymbol::BytesLen => "kata_rt_bytes_len",
            FfiSymbol::BytesGet => "kata_rt_bytes_get",
            FfiSymbol::BytesSet => "kata_rt_bytes_set",
            FfiSymbol::BytesGetChecked => "kata_rt_bytes_get_checked",
            FfiSymbol::BytesConcat => "kata_rt_bytes_concat",
            FfiSymbol::BytesEq => "kata_rt_bytes_eq",
            FfiSymbol::BytesNeq => "kata_rt_bytes_neq",
            FfiSymbol::BytesShow => "kata_rt_bytes_show",
            FfiSymbol::BytesSlice => "kata_rt_bytes_slice",
            FfiSymbol::BytesAnd => "kata_rt_bytes_and",
            FfiSymbol::BytesOr => "kata_rt_bytes_or",
            FfiSymbol::BytesXor => "kata_rt_bytes_xor",
            FfiSymbol::BytesNot => "kata_rt_bytes_not",
            FfiSymbol::ByteAnd => "kata_rt_byte_and",
            FfiSymbol::ByteOr => "kata_rt_byte_or",
            FfiSymbol::ByteXor => "kata_rt_byte_xor",
            FfiSymbol::ByteNot => "kata_rt_byte_not",
            FfiSymbol::ByteShr => "kata_rt_byte_shr",
            FfiSymbol::ByteShl => "kata_rt_byte_shl",
            FfiSymbol::ByteToInt => "kata_rt_byte_to_int",
            FfiSymbol::IntToByte => "kata_rt_int_to_byte",
            FfiSymbol::IntToBytes => "kata_rt_int_to_bytes",
            FfiSymbol::TextToBytes => "kata_rt_text_to_bytes",
            FfiSymbol::BytesToText => "kata_rt_bytes_to_text",
            FfiSymbol::TextAt => "kata_rt_text_at",
            FfiSymbol::TextLen => "kata_rt_text_len",
            FfiSymbol::TextSlice => "kata_rt_text_slice",
            FfiSymbol::ArraySlice => "kata_rt_array_slice",
            FfiSymbol::ListSlice => "kata_rt_list_slice",
            FfiSymbol::ToBytes => "kata_rt_to_bytes",
            FfiSymbol::FromBytes => "kata_rt_from_bytes",
            // File I/O
            FfiSymbol::FileOpen => "kata_rt_file_open",
            FfiSymbol::FileRead => "kata_rt_file_read",
            FfiSymbol::FileReadChunk => "kata_rt_file_read_chunk",
            FfiSymbol::FileReadline => "kata_rt_file_readline",
            FfiSymbol::FileWriteText => "kata_rt_file_write_text",
            FfiSymbol::FileWriteBytes => "kata_rt_file_write_bytes",
            FfiSymbol::FileClose => "kata_rt_file_close",
            // Socket I/O
            FfiSymbol::SocketOpen => "kata_rt_socket_open",
            FfiSymbol::SocketListen => "kata_rt_socket_listen",
            FfiSymbol::SocketRead => "kata_rt_socket_read",
            FfiSymbol::SocketReadChunk => "kata_rt_socket_read_chunk",
            FfiSymbol::SocketReadline => "kata_rt_socket_readline",
            FfiSymbol::SocketWriteText => "kata_rt_socket_write_text",
            FfiSymbol::SocketWriteBytes => "kata_rt_socket_write_bytes",
            FfiSymbol::SocketClose => "kata_rt_socket_close",
            // Timer
            FfiSymbol::TimerNow => "kata_rt_timer_now",
        }
    }

    /// Tipo de retorno do símbolo FFI.
    pub fn return_type(self) -> Ty {
        match self {
            // Aritmética Int → Int
            FfiSymbol::BiAdd | FfiSymbol::BiSub | FfiSymbol::BiMul | FfiSymbol::BiDiv => Ty::int(),
            // Comparação Int → Boolean
            FfiSymbol::BiEq
            | FfiSymbol::BiNeq
            | FfiSymbol::BiLt
            | FfiSymbol::BiLe
            | FfiSymbol::BiGt
            | FfiSymbol::BiGe => Ty::boolean(),
            FfiSymbol::BiShow | FfiSymbol::BiToRational => Ty::text(),
            FfiSymbol::TagInt => Ty::int(),
            // Float
            FfiSymbol::Fadd | FfiSymbol::Fsub | FfiSymbol::Fmul | FfiSymbol::Fdiv => Ty::float(),
            FfiSymbol::FcmpEq
            | FfiSymbol::FcmpNeq
            | FfiSymbol::FcmpLt
            | FfiSymbol::FcmpLe
            | FfiSymbol::FcmpGt
            | FfiSymbol::FcmpGe => Ty::boolean(),
            FfiSymbol::FloatToText => Ty::text(),
            // Rational
            FfiSymbol::RatAdd | FfiSymbol::RatSub | FfiSymbol::RatMul | FfiSymbol::RatDiv => {
                Ty::rational()
            }
            FfiSymbol::RatEq
            | FfiSymbol::RatNeq
            | FfiSymbol::RatLt
            | FfiSymbol::RatLe
            | FfiSymbol::RatGt
            | FfiSymbol::RatGe => Ty::boolean(),
            FfiSymbol::RatShow | FfiSymbol::RatToFloat => Ty::text(),
            FfiSymbol::RatFromFloat | FfiSymbol::RatLiteral => Ty::rational(),
            FfiSymbol::IntToRational => Ty::rational(),
            // Text
            FfiSymbol::StringConcat => Ty::text(),
            FfiSymbol::StringLen => Ty::int(),
            FfiSymbol::TextLiteral => Ty::text(),
            FfiSymbol::IntToText | FfiSymbol::BoolToText => Ty::text(),
            FfiSymbol::TextReplaceFirst => Ty::text(),
            // I/O
            FfiSymbol::Print | FfiSymbol::Println => Ty::Unit,
            // Arena
            FfiSymbol::ArenaCreate | FfiSymbol::ArenaAlloc | FfiSymbol::ArenaCreateTracked => {
                Ty::int()
            }
            FfiSymbol::ArenaDestroy | FfiSymbol::ArenaDealloc => Ty::Unit,
            FfiSymbol::GetRootArenaHandle => Ty::int(),
            FfiSymbol::ArenaStats => Ty::int(),
            // Sum
            FfiSymbol::StoreSumResult | FfiSymbol::SumTagInt => Ty::int(),
            // Control flow — panic retorna Unit (aborta antes, mas o tipo é Unit)
            FfiSymbol::Panic => Ty::Unit,
            // Scheduler/Fiber
            FfiSymbol::SchedulerInit => Ty::int(),
            FfiSymbol::Spawn => Ty::int(),
            FfiSymbol::SpawnProcess => Ty::int(), // value_ptr do resultado desserializado
            FfiSymbol::Run => Ty::int(),
            FfiSymbol::Yield => Ty::Unit,
            FfiSymbol::YieldCheck => Ty::Unit,
            FfiSymbol::SetTestTimeout => Ty::Unit,
            FfiSymbol::Sleep => Ty::Unit,
            // Arc<T> / CaptureBox
            FfiSymbol::AllocArc | FfiSymbol::ArcFnPtr => Ty::int(),
            FfiSymbol::IncRef | FfiSymbol::DecRef => Ty::int(),
            // Collections — todas retornam I64 (ptr ou valor i64)
            FfiSymbol::ListNil => Ty::int(),
            FfiSymbol::ListCons => Ty::int(),
            FfiSymbol::ListIsEmpty => Ty::boolean(),
            FfiSymbol::ListHead => Ty::int(),
            FfiSymbol::ListTail => Ty::int(),
            FfiSymbol::ListLen => Ty::int(),
            FfiSymbol::ListGetChecked => Ty::int(),
            FfiSymbol::ArrayAlloc => Ty::int(),
            FfiSymbol::ArrayLen => Ty::int(),
            FfiSymbol::ArrayGet => Ty::int(),
            FfiSymbol::ArraySet => Ty::Unit,
            FfiSymbol::ArrayGetChecked => Ty::int(),
            FfiSymbol::RangeAlloc => Ty::int(),
            FfiSymbol::ListContains => Ty::boolean(),
            FfiSymbol::ArrayContains => Ty::boolean(),
            FfiSymbol::ListReverse => Ty::int(),
            FfiSymbol::ListConcat => Ty::int(),
            // Hash (Fio 13) — todas retornam i64 (hash)
            FfiSymbol::HashInt | FfiSymbol::HashText | FfiSymbol::HashRational => Ty::int(),
            // Dict (Fio 13) — todas retornam i64 (ptr ou bool)
            FfiSymbol::DictEmpty | FfiSymbol::DictInsert | FfiSymbol::DictRemove => Ty::int(),
            FfiSymbol::DictGetChecked => Ty::int(),
            FfiSymbol::DictContains => Ty::boolean(),
            FfiSymbol::DictLen => Ty::int(),
            FfiSymbol::DictNext => Ty::int(),
            // Set (Fio 13)
            FfiSymbol::SetEmpty | FfiSymbol::SetInsert | FfiSymbol::SetRemove => Ty::int(),
            FfiSymbol::SetContains => Ty::boolean(),
            FfiSymbol::SetLen => Ty::int(),
            FfiSymbol::SetNext => Ty::int(),
            FfiSymbol::SetUnion | FfiSymbol::SetIntersection | FfiSymbol::SetDifference => {
                Ty::int()
            }
            FfiSymbol::DictMerge => Ty::int(),
            // String equality (Fio 13) — retorna 0/1
            FfiSymbol::StringEq => Ty::boolean(),
            // Canais CSP — handles são i64 (ponteiro+tag)
            FfiSymbol::ChannelCreate => Ty::int(),
            FfiSymbol::QueueCreate => Ty::int(),
            FfiSymbol::BroadcastCreate => Ty::int(),
            FfiSymbol::BroadcastReceiverCreate => Ty::int(),
            FfiSymbol::IpcChannelCreate => Ty::int(),
            FfiSymbol::IpcQueueCreate => Ty::int(),
            FfiSymbol::ChannelSend => Ty::int(),
            FfiSymbol::ChannelRecv => Ty::int(),
            FfiSymbol::ChannelSelect => Ty::int(),
            FfiSymbol::SelectFiles => Ty::int(),
            FfiSymbol::SelectCombined => Ty::int(),
            // Log — LogPublish/LogRecv retornam i64 (status/valor), LogConfig retorna Unit
            FfiSymbol::LogPublish => Ty::int(),
            FfiSymbol::LogRecv => Ty::int(),
            FfiSymbol::LogConfig => Ty::Unit,
            // Comptime snapshots
            FfiSymbol::LoadSnapshot => Ty::Unit,
            FfiSymbol::GetSnapshot => Ty::int(),
            // Cache
            FfiSymbol::CacheGetOrCreate => Ty::int(),
            FfiSymbol::CacheLookup => Ty::int(),
            FfiSymbol::CacheInsert => Ty::Unit,
            FfiSymbol::CacheSerializeKey => Ty::int(),
            // Bytes / Byte (PRD-bytes)
            FfiSymbol::BytesAlloc
            | FfiSymbol::BytesFromPtr
            | FfiSymbol::BytesFromInts
            | FfiSymbol::BytesConcat
            | FfiSymbol::BytesSlice
            | FfiSymbol::BytesAnd
            | FfiSymbol::BytesOr
            | FfiSymbol::BytesXor
            | FfiSymbol::BytesNot
            | FfiSymbol::IntToBytes
            | FfiSymbol::TextToBytes => Ty::Bytes,
            FfiSymbol::BytesLen => Ty::int(),
            FfiSymbol::BytesGet => Ty::Byte,
            FfiSymbol::BytesSet => Ty::Unit,
            FfiSymbol::BytesGetChecked => Ty::int(),
            FfiSymbol::BytesEq | FfiSymbol::BytesNeq => Ty::boolean(),
            FfiSymbol::BytesShow | FfiSymbol::BytesToText | FfiSymbol::TextSlice => Ty::text(),
            FfiSymbol::ByteAnd
            | FfiSymbol::ByteOr
            | FfiSymbol::ByteXor
            | FfiSymbol::ByteNot
            | FfiSymbol::ByteShr
            | FfiSymbol::ByteShl
            | FfiSymbol::ByteToInt
            | FfiSymbol::IntToByte => Ty::int(),
            FfiSymbol::TextAt => Ty::int(),
            FfiSymbol::TextLen => Ty::int(),
            FfiSymbol::ArraySlice | FfiSymbol::ListSlice => Ty::int(),
            FfiSymbol::ToBytes => Ty::Bytes,
            FfiSymbol::FromBytes => Ty::int(), // ponteiro genérico (tipo depende do contexto)
            // File I/O — retornam i64 (Result box ARC tracked)
            FfiSymbol::FileOpen => Ty::int(),      // Result box ptr
            FfiSymbol::FileRead => Ty::int(),      // Result box ptr
            FfiSymbol::FileReadChunk => Ty::int(), // Result box ptr
            FfiSymbol::FileReadline => Ty::int(),  // Result box ptr
            FfiSymbol::FileWriteText => Ty::int(), // Result box ptr
            FfiSymbol::FileWriteBytes => Ty::int(), // Result box ptr
            FfiSymbol::FileClose => Ty::Unit,
            // Socket I/O — retornam i64 (Result box ptr) ou Unit (close)
            FfiSymbol::SocketOpen => Ty::int(),
            FfiSymbol::SocketListen => Ty::int(),
            FfiSymbol::SocketRead => Ty::int(),
            FfiSymbol::SocketReadChunk => Ty::int(),
            FfiSymbol::SocketReadline => Ty::int(),
            FfiSymbol::SocketWriteText => Ty::int(),
            FfiSymbol::SocketWriteBytes => Ty::int(),
            FfiSymbol::SocketClose => Ty::Unit,
            // Timer
            FfiSymbol::TimerNow => Ty::int(),
        }
    }

    /// Constrói FfiSymbol a partir do nome do símbolo C.
    pub fn from_name(name: &str) -> Option<FfiSymbol> {
        let all = [
            FfiSymbol::BiAdd,
            FfiSymbol::BiSub,
            FfiSymbol::BiMul,
            FfiSymbol::BiDiv,
            FfiSymbol::BiEq,
            FfiSymbol::BiNeq,
            FfiSymbol::BiLt,
            FfiSymbol::BiLe,
            FfiSymbol::BiGt,
            FfiSymbol::BiGe,
            FfiSymbol::BiShow,
            FfiSymbol::BiToRational,
            FfiSymbol::TagInt,
            FfiSymbol::Fadd,
            FfiSymbol::Fsub,
            FfiSymbol::Fmul,
            FfiSymbol::Fdiv,
            FfiSymbol::FcmpEq,
            FfiSymbol::FcmpNeq,
            FfiSymbol::FcmpLt,
            FfiSymbol::FcmpLe,
            FfiSymbol::FcmpGt,
            FfiSymbol::FcmpGe,
            FfiSymbol::FloatToText,
            FfiSymbol::RatAdd,
            FfiSymbol::RatSub,
            FfiSymbol::RatMul,
            FfiSymbol::RatDiv,
            FfiSymbol::RatEq,
            FfiSymbol::RatNeq,
            FfiSymbol::RatLt,
            FfiSymbol::RatLe,
            FfiSymbol::RatGt,
            FfiSymbol::RatGe,
            FfiSymbol::RatShow,
            FfiSymbol::RatToFloat,
            FfiSymbol::RatFromFloat,
            FfiSymbol::RatLiteral,
            FfiSymbol::IntToRational,
            FfiSymbol::StringConcat,
            FfiSymbol::StringLen,
            FfiSymbol::TextLiteral,
            FfiSymbol::IntToText,
            FfiSymbol::BoolToText,
            FfiSymbol::TextReplaceFirst,
            FfiSymbol::Print,
            FfiSymbol::Println,
            FfiSymbol::ArenaCreate,
            FfiSymbol::ArenaAlloc,
            FfiSymbol::ArenaDestroy,
            FfiSymbol::ArenaCreateTracked,
            FfiSymbol::ArenaDealloc,
            FfiSymbol::GetRootArenaHandle,
            FfiSymbol::ArenaStats,
            FfiSymbol::StoreSumResult,
            FfiSymbol::SumTagInt,
            FfiSymbol::Panic,
            FfiSymbol::SchedulerInit,
            FfiSymbol::Spawn,
            FfiSymbol::SpawnProcess,
            FfiSymbol::Run,
            FfiSymbol::Yield,
            FfiSymbol::YieldCheck,
            FfiSymbol::SetTestTimeout,
            FfiSymbol::Sleep,
            FfiSymbol::AllocArc,
            FfiSymbol::IncRef,
            FfiSymbol::DecRef,
            FfiSymbol::ArcFnPtr,
            // Collections
            FfiSymbol::ListNil,
            FfiSymbol::ListCons,
            FfiSymbol::ListIsEmpty,
            FfiSymbol::ListHead,
            FfiSymbol::ListTail,
            FfiSymbol::ListLen,
            FfiSymbol::ListGetChecked,
            FfiSymbol::ArrayAlloc,
            FfiSymbol::ArrayLen,
            FfiSymbol::ArrayGet,
            FfiSymbol::ArraySet,
            FfiSymbol::ArrayGetChecked,
            FfiSymbol::RangeAlloc,
            FfiSymbol::ListContains,
            FfiSymbol::ArrayContains,
            FfiSymbol::ListReverse,
            FfiSymbol::ListConcat,
            // Hash (Fio 13)
            FfiSymbol::HashInt,
            FfiSymbol::HashText,
            FfiSymbol::HashRational,
            // Dict (Fio 13)
            FfiSymbol::DictEmpty,
            FfiSymbol::DictInsert,
            FfiSymbol::DictGetChecked,
            FfiSymbol::DictContains,
            FfiSymbol::DictLen,
            FfiSymbol::DictRemove,
            FfiSymbol::DictNext,
            // Set (Fio 13)
            FfiSymbol::SetEmpty,
            FfiSymbol::SetInsert,
            FfiSymbol::SetContains,
            FfiSymbol::SetLen,
            FfiSymbol::SetRemove,
            FfiSymbol::SetNext,
            FfiSymbol::SetUnion,
            FfiSymbol::SetIntersection,
            FfiSymbol::SetDifference,
            FfiSymbol::DictMerge,
            // String equality (Fio 13)
            FfiSymbol::StringEq,
            // Canais CSP
            FfiSymbol::ChannelCreate,
            FfiSymbol::QueueCreate,
            FfiSymbol::BroadcastCreate,
            FfiSymbol::BroadcastReceiverCreate,
            FfiSymbol::IpcChannelCreate,
            FfiSymbol::IpcQueueCreate,
            FfiSymbol::ChannelSend,
            FfiSymbol::ChannelRecv,
            FfiSymbol::ChannelSelect,
            FfiSymbol::SelectFiles,
            FfiSymbol::SelectCombined,
            // Log
            FfiSymbol::LogPublish,
            FfiSymbol::LogRecv,
            FfiSymbol::LogConfig,
            // Comptime snapshots
            FfiSymbol::LoadSnapshot,
            FfiSymbol::GetSnapshot,
            // Cache
            FfiSymbol::CacheGetOrCreate,
            FfiSymbol::CacheLookup,
            FfiSymbol::CacheInsert,
            FfiSymbol::CacheSerializeKey,
            // Bytes / Byte (PRD-bytes)
            FfiSymbol::BytesAlloc,
            FfiSymbol::BytesFromPtr,
            FfiSymbol::BytesFromInts,
            FfiSymbol::BytesLen,
            FfiSymbol::BytesGet,
            FfiSymbol::BytesSet,
            FfiSymbol::BytesGetChecked,
            FfiSymbol::BytesConcat,
            FfiSymbol::BytesEq,
            FfiSymbol::BytesNeq,
            FfiSymbol::BytesShow,
            FfiSymbol::BytesSlice,
            FfiSymbol::BytesAnd,
            FfiSymbol::BytesOr,
            FfiSymbol::BytesXor,
            FfiSymbol::BytesNot,
            FfiSymbol::ByteAnd,
            FfiSymbol::ByteOr,
            FfiSymbol::ByteXor,
            FfiSymbol::ByteNot,
            FfiSymbol::ByteShr,
            FfiSymbol::ByteShl,
            FfiSymbol::ByteToInt,
            FfiSymbol::IntToByte,
            FfiSymbol::IntToBytes,
            FfiSymbol::TextToBytes,
            FfiSymbol::BytesToText,
            FfiSymbol::TextAt,
            FfiSymbol::TextLen,
            FfiSymbol::TextSlice,
            FfiSymbol::ArraySlice,
            FfiSymbol::ListSlice,
            FfiSymbol::ToBytes,
            FfiSymbol::FromBytes,
            // File I/O
            FfiSymbol::FileOpen,
            FfiSymbol::FileRead,
            FfiSymbol::FileReadChunk,
            FfiSymbol::FileReadline,
            FfiSymbol::FileWriteText,
            FfiSymbol::FileWriteBytes,
            FfiSymbol::FileClose,
            // Socket I/O
            FfiSymbol::SocketOpen,
            FfiSymbol::SocketListen,
            FfiSymbol::SocketRead,
            FfiSymbol::SocketReadChunk,
            FfiSymbol::SocketReadline,
            FfiSymbol::SocketWriteText,
            FfiSymbol::SocketWriteBytes,
            FfiSymbol::SocketClose,
            // Timer
            FfiSymbol::TimerNow,
        ];
        all.iter().copied().find(|s| s.symbol_name() == name)
    }
}
