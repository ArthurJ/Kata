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

    // ── Canais CSP ────────────────────────────
    /// `kata_rt_channel_create(arena) -> i64` — canal rendezvous.
    ChannelCreate,
    /// `kata_rt_queue_create(arena, capacity) -> i64` — fila bufferizada.
    QueueCreate,
    /// `kata_rt_broadcast_create(arena) -> i64` — broadcast pub-sub.
    BroadcastCreate,
    /// `kata_rt_broadcast_receiver_create(arena, factory) -> i64` — receiver.
    BroadcastReceiverCreate,
    /// `kata_rt_channel_send(handle, value) -> i64` — envia (0=OK, -1=block).
    ChannelSend,
    /// `kata_rt_channel_recv(handle) -> i64` — recebe (valor ou -1=block).
    ChannelRecv,
    /// `kata_rt_select(handles, n) -> i64` — select multiplex.
    ChannelSelect,
    /// `kata_rt_log_publish(topic_ptr, level, msg, policy_ptr) -> i64` — publica msg no tópico.
    LogPublish,
    /// `kata_rt_log_recv(topic_ptr) -> i64` — recebe próxima mensagem do tópico.
    LogRecv,
    /// `kata_rt_log_config(topic_ptr, policy_ptr, level) -> ()` — setta defaults de logging.
    LogConfig,
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
            FfiSymbol::StoreSumResult => "kata_rt_store_sum_result",
            FfiSymbol::SumTagInt => "kata_rt_sum_tag_int",
            FfiSymbol::Panic => "kata_rt_panic",
            FfiSymbol::SchedulerInit => "kata_rt_scheduler_init",
            FfiSymbol::Spawn => "kata_rt_spawn",
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
            // Canais CSP
            FfiSymbol::ChannelCreate => "kata_rt_channel_create",
            FfiSymbol::QueueCreate => "kata_rt_queue_create",
            FfiSymbol::BroadcastCreate => "kata_rt_broadcast_create",
            FfiSymbol::BroadcastReceiverCreate => "kata_rt_broadcast_receiver_create",
            FfiSymbol::ChannelSend => "kata_rt_channel_send",
            FfiSymbol::ChannelRecv => "kata_rt_channel_recv",
            FfiSymbol::ChannelSelect => "kata_rt_select",
            // Log
            FfiSymbol::LogPublish => "kata_rt_log_publish",
            FfiSymbol::LogRecv => "kata_rt_log_recv",
            FfiSymbol::LogConfig => "kata_rt_log_config",
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
            FfiSymbol::ArenaCreate | FfiSymbol::ArenaAlloc => Ty::int(),
            FfiSymbol::ArenaDestroy => Ty::Unit,
            // Sum
            FfiSymbol::StoreSumResult | FfiSymbol::SumTagInt => Ty::int(),
            // Control flow — panic retorna Unit (aborta antes, mas o tipo é Unit)
            FfiSymbol::Panic => Ty::Unit,
            // Scheduler/Fiber
            FfiSymbol::SchedulerInit => Ty::int(),
            FfiSymbol::Spawn => Ty::int(),
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
            // Canais CSP — handles são i64 (ponteiro+tag)
            FfiSymbol::ChannelCreate => Ty::int(),
            FfiSymbol::QueueCreate => Ty::int(),
            FfiSymbol::BroadcastCreate => Ty::int(),
            FfiSymbol::BroadcastReceiverCreate => Ty::int(),
            FfiSymbol::ChannelSend => Ty::int(),
            FfiSymbol::ChannelRecv => Ty::int(),
            FfiSymbol::ChannelSelect => Ty::int(),
            // Log — LogPublish/LogRecv retornam i64 (status/valor), LogConfig retorna Unit
            FfiSymbol::LogPublish => Ty::int(),
            FfiSymbol::LogRecv => Ty::int(),
            FfiSymbol::LogConfig => Ty::Unit,
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
            FfiSymbol::StoreSumResult,
            FfiSymbol::SumTagInt,
            FfiSymbol::Panic,
            FfiSymbol::SchedulerInit,
            FfiSymbol::Spawn,
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
            // Canais CSP
            FfiSymbol::ChannelCreate,
            FfiSymbol::QueueCreate,
            FfiSymbol::BroadcastCreate,
            FfiSymbol::BroadcastReceiverCreate,
            FfiSymbol::ChannelSend,
            FfiSymbol::ChannelRecv,
            FfiSymbol::ChannelSelect,
            // Log
            FfiSymbol::LogPublish,
            FfiSymbol::LogRecv,
            FfiSymbol::LogConfig,
        ];
        all.iter().copied().find(|s| s.symbol_name() == name)
    }
}
