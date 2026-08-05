//! Catálogo de assinaturas FFI para Cranelift.
//!
//! Mapeia cada `FfiSymbol` para uma `Signature` Cranelift com `AbiParam`
//! tipados corretamente. O codegen usa isto para declarar imports no
//! `JITModule` e `FunctionBuilder`.

use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ffi::FfiSymbol;
use kata_core::ty::{PrimTy, Ty};

/// Mapeia `Ty` para o tipo Cranelift correspondente na ABI.
///
/// Int → I64 (SMI tagging: o compilador vê i64 em todo o pipeline).
/// Float → F64.
/// Text/Rational/Struct/Sum → I64 (ponteiro opaco).
/// Unit → sem retorno (void).
/// Function → I64 (function pointer — fios posteriores).
/// InferVar → I64 (fallback graceful — não deveria chegar aqui).
pub(crate) fn ty_to_clif(ty: &Ty) -> cranelift_codegen::ir::Type {
    match ty {
        Ty::Prim(PrimTy::Int) => I64,
        Ty::Prim(PrimTy::Float) => F64,
        Ty::Prim(PrimTy::Text) | Ty::Prim(PrimTy::Rational) => I64,
        Ty::Unit => I64, // Unit é representado como I64 zero
        Ty::Struct(_)
        | Ty::Sum(_)
        | Ty::Tuple(_)
        | Ty::List(_)
        | Ty::Array(_)
        | Ty::Range(_)
        | Ty::Dict(_, _)
        | Ty::Set(_)
        | Ty::Sender(_)
        | Ty::Receiver(_)
        | Ty::ReceiverFactory(_)
        | Ty::Bytes
        | Ty::File
        | Ty::Socket => I64,
        Ty::Byte => I64,
        Ty::Function(_, _) => I64,
        Ty::Action(_, _) => I64,
        Ty::InferVar(_) => I64,
        // Var e Generic: Sum é sempre ponteiro opaco (box tag+payload).
        Ty::Var(_) => I64,
        Ty::Generic(_, _) => I64,
        // Interface: não é tipo concreto — não deveria chegar ao codegen.
        // Mapeia para I64 como fallback graceful.
        Ty::Interface(_) => I64,
    }
}

/// Constrói a assinatura Cranelift para um `FfiSymbol`.
///
/// Match exaustivo — se uma variante nova for adicionada ao enum
/// `FfiSymbol` sem assinatura aqui, o compilador Rust emite erro.
pub(crate) fn ffi_signature(sym: FfiSymbol) -> Signature {
    let mut sig = Signature::new(CallConv::SystemV);

    match sym {
        // ── Aritmética Int (i64, i64) → i64 ──
        FfiSymbol::BiAdd | FfiSymbol::BiSub | FfiSymbol::BiMul | FfiSymbol::BiDiv => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Comparação Int (i64, i64) → i64 (0/1) ──
        FfiSymbol::BiEq
        | FfiSymbol::BiNeq
        | FfiSymbol::BiLt
        | FfiSymbol::BiLe
        | FfiSymbol::BiGt
        | FfiSymbol::BiGe => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Show Int → Text (i64) → i64 (ptr) ──
        FfiSymbol::BiShow => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Int → Rational (i64) → i64 (ptr) ──
        FfiSymbol::BiToRational => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Tagging: i64 cru → i64 SMI/BigInt ──
        FfiSymbol::TagInt => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Aritmética Float (f64, f64) → f64 ──
        FfiSymbol::Fadd | FfiSymbol::Fsub | FfiSymbol::Fmul | FfiSymbol::Fdiv => {
            sig.params.push(AbiParam::new(F64));
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(F64));
        }
        // ── Comparação Float (f64, f64) → i64 (0/1) ──
        FfiSymbol::FcmpEq
        | FfiSymbol::FcmpNeq
        | FfiSymbol::FcmpLt
        | FfiSymbol::FcmpLe
        | FfiSymbol::FcmpGt
        | FfiSymbol::FcmpGe => {
            sig.params.push(AbiParam::new(F64));
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Float → Text (f64) → i64 (ptr) ──
        FfiSymbol::FloatToText => {
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Aritmética Rational (ptr, ptr) → ptr ──
        FfiSymbol::RatAdd | FfiSymbol::RatSub | FfiSymbol::RatMul | FfiSymbol::RatDiv => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Comparação Rational (ptr, ptr) → i64 (0/1) ──
        FfiSymbol::RatEq
        | FfiSymbol::RatNeq
        | FfiSymbol::RatLt
        | FfiSymbol::RatLe
        | FfiSymbol::RatGt
        | FfiSymbol::RatGe => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Show Rational → Text (ptr) → i64 (ptr) ──
        FfiSymbol::RatShow => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Rational → Float (ptr) → f64 ──
        FfiSymbol::RatToFloat => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(F64));
        }
        // ── Float → Rational (f64) → i64 (ptr) ──
        FfiSymbol::RatFromFloat => {
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Literal Rational (ptr texto, len) → ptr ──
        // RatLiteral recebe ponteiro para string C + length.
        FfiSymbol::RatLiteral => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // len
            sig.returns.push(AbiParam::new(I64)); // ptr Rational
        }
        // ── Int → Rational (i64 tagged) → i64 (ptr) ──
        FfiSymbol::IntToRational => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text concat (ptr, ptr) → ptr ──
        FfiSymbol::StringConcat => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text len (ptr) → i64 ──
        FfiSymbol::StringLen => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text literal (ptr, len) → ptr ──
        FfiSymbol::TextLiteral => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // len
            sig.returns.push(AbiParam::new(I64)); // ptr string
        }
        // ── Int → Text (i64 tagged) → ptr ──
        FfiSymbol::IntToText => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Boolean → Text (i64 0/1) → ptr ──
        FfiSymbol::BoolToText => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text replace first (ptr, ptr) → ptr ──
        FfiSymbol::TextReplaceFirst => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── I/O (ptr) → void ──
        FfiSymbol::Print | FfiSymbol::Println => {
            sig.params.push(AbiParam::new(I64));
        }
        // ── Arena (void → ptr, ptr,size → ptr, ptr → void) ──
        FfiSymbol::ArenaCreate => {
            sig.returns.push(AbiParam::new(I64));
        }
        FfiSymbol::ArenaAlloc => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // size
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        FfiSymbol::ArenaDestroy => {
            sig.params.push(AbiParam::new(I64)); // arena
        }
        FfiSymbol::ArenaCreateTracked => {
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        FfiSymbol::ArenaDealloc => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // size
        }
        FfiSymbol::GetRootArenaHandle => {
            sig.returns.push(AbiParam::new(I64)); // root_arena handle
        }
        FfiSymbol::ArenaStats => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // packed (alloc_count, dealloc_count)
        }
        // ── Sum (i64, i64, i64) → i64, (i64) → i64 ──
        // Pré-11: store_sum_result recebe arena_handle como 3º param.
        FfiSymbol::StoreSumResult => {
            sig.params.push(AbiParam::new(I64)); // tag
            sig.params.push(AbiParam::new(I64)); // payload
            sig.params.push(AbiParam::new(I64)); // arena_handle
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        FfiSymbol::SumTagInt => {
            sig.params.push(AbiParam::new(I64)); // val (ptr to box)
            sig.returns.push(AbiParam::new(I64)); // tag
        }
        // ── Control flow (ptr) → void (never returns) ──
        FfiSymbol::Panic => {
            sig.params.push(AbiParam::new(I64)); // msg ptr
        }
        // ── Scheduler/Fiber ──
        // scheduler_init: () -> i64 (1 = sucesso)
        FfiSymbol::SchedulerInit => {
            sig.returns.push(AbiParam::new(I64));
        }
        // spawn: (fn_ptr: i64, caller_arena: i64, args_ptr: i64) -> i64 (fiber_id)
        FfiSymbol::Spawn => {
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // caller_arena
            sig.params.push(AbiParam::new(I64)); // args_ptr
            sig.returns.push(AbiParam::new(I64)); // fiber_id
        }
        // run: () -> i64 (resultado do fiber)
        FfiSymbol::Run => {
            sig.returns.push(AbiParam::new(I64));
        }
        // yield: () → void (suspende fiber)
        FfiSymbol::Yield => {}
        // yield_check: () → void (yield point no header de loops, )
        FfiSymbol::YieldCheck => {}
        // set_test_timeout: (millis: i64) → void (configura timer de teste)
        // Chamada pelo runner antes de kata_rt_run.
        FfiSymbol::SetTestTimeout => {
            sig.params.push(AbiParam::new(I64)); // millis
        }
        // sleep: (ms: i64) → void (sleep cooperativo, suspende fiber)
        FfiSymbol::Sleep => {
            sig.params.push(AbiParam::new(I64)); // ms (SMI-tagged)
        }
        // ── Arc<T> / CaptureBox ──
        // alloc_arc: (fn_ptr, captures_ptr, n_captures, arena_handle) -> box_ptr
        // Pré-11: arena_handle adicionado como 4º param.
        FfiSymbol::AllocArc => {
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // captures_ptr
            sig.params.push(AbiParam::new(I64)); // n_captures
            sig.params.push(AbiParam::new(I64)); // arena_handle
            sig.returns.push(AbiParam::new(I64)); // box_ptr
        }
        // incref: (box_ptr) -> 0
        FfiSymbol::IncRef => {
            sig.params.push(AbiParam::new(I64)); // box_ptr
            sig.returns.push(AbiParam::new(I64));
        }
        // decref: (box_ptr) -> 0
        FfiSymbol::DecRef => {
            sig.params.push(AbiParam::new(I64)); // box_ptr
            sig.returns.push(AbiParam::new(I64));
        }
        // arc_fn_ptr: (box_ptr) -> fn_ptr
        FfiSymbol::ArcFnPtr => {
            sig.params.push(AbiParam::new(I64)); // box_ptr
            sig.returns.push(AbiParam::new(I64)); // fn_ptr
        }
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
        // ── String equality (Fio 13) ──
        // string_eq: (a, b) -> i64 (0/1)
        FfiSymbol::StringEq => {
            sig.params.push(AbiParam::new(I64)); // a (str ptr)
            sig.params.push(AbiParam::new(I64)); // b (str ptr)
            sig.returns.push(AbiParam::new(I64)); // bool (0/1)
        }
        // ── Canais CSP ──
        // channel_create: (arena) -> handle
        FfiSymbol::ChannelCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // queue_create: (arena, capacity) -> handle
        FfiSymbol::QueueCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // capacity
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // broadcast_create: (arena) -> handle
        FfiSymbol::BroadcastCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // broadcast_receiver_create: (arena, factory_handle) -> handle
        FfiSymbol::BroadcastReceiverCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // factory_handle
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // ipc_channel_create: (arena, type_id, ack_tx_handle) -> handle
        FfiSymbol::IpcChannelCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // type_id
            sig.params.push(AbiParam::new(I64)); // ack_tx_handle
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // ipc_queue_create: (arena, cap, type_id) -> ptr (6 handles na arena)
        FfiSymbol::IpcQueueCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // cap
            sig.params.push(AbiParam::new(I64)); // type_id
            sig.returns.push(AbiParam::new(I64)); // ptr to 6-handle tuple
        }
        // channel_send: (handle, value) -> i64 (0=OK, -1=block)
        FfiSymbol::ChannelSend => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // value
            sig.returns.push(AbiParam::new(I64)); // status
        }
        // channel_recv: (handle) -> i64 (valor ou -1=block)
        FfiSymbol::ChannelRecv => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // value
        }
        // select: (handles_ptr, n_handles, timeout_ms) -> i64 (idx, -1=block, -2=timeout)
        FfiSymbol::ChannelSelect => {
            sig.params.push(AbiParam::new(I64)); // handles ptr
            sig.params.push(AbiParam::new(I64)); // n_handles
            sig.params.push(AbiParam::new(I64)); // timeout_ms (<=0 = sem timeout)
            sig.returns.push(AbiParam::new(I64)); // index or sentinel
        }
        // select_files: (handles_ptr, n_handles) -> i64 (idx, -1=block)
        FfiSymbol::SelectFiles => {
            sig.params.push(AbiParam::new(I64)); // handles ptr
            sig.params.push(AbiParam::new(I64)); // n_handles
            sig.returns.push(AbiParam::new(I64)); // index or sentinel
        }
        // select_combined: (chan_ptr, n_c, file_ptr, n_f, socket_ptr, n_s, timeout_ms) -> i64
        // Retorna índice global: 0..n_c-1 = channel, n_c..n_c+n_f-1 = file,
        // n_c+n_f..n_c+n_f+n_s-1 = socket.
        // -1 = WOULD_BLOCK, -2 = SELECT_TIMEOUT.
        FfiSymbol::SelectCombined => {
            sig.params.push(AbiParam::new(I64)); // chan_handles ptr
            sig.params.push(AbiParam::new(I64)); // n_c
            sig.params.push(AbiParam::new(I64)); // file_handles ptr
            sig.params.push(AbiParam::new(I64)); // n_f
            sig.params.push(AbiParam::new(I64)); // socket_handles ptr
            sig.params.push(AbiParam::new(I64)); // n_s
            sig.params.push(AbiParam::new(I64)); // timeout_ms (<=0 = sem timeout)
            sig.returns.push(AbiParam::new(I64)); // global index or sentinel
        }
        // log_publish: (topic_ptr, level, msg, policy_ptr) -> i64 (0=OK, -1=erro)
        FfiSymbol::LogPublish => {
            sig.params.push(AbiParam::new(I64)); // topic_ptr (handle Text ou 0)
            sig.params.push(AbiParam::new(I64)); // level (tag do enum LogLevel)
            sig.params.push(AbiParam::new(I64)); // msg (handle Text)
            sig.params.push(AbiParam::new(I64)); // policy_ptr (handle Text ou 0)
            sig.returns.push(AbiParam::new(I64)); // status
        }
        // log_recv: (topic_ptr) -> i64 (valor ou 0 se canal fechou)
        FfiSymbol::LogRecv => {
            sig.params.push(AbiParam::new(I64)); // topic_ptr (handle Text ou 0)
            sig.returns.push(AbiParam::new(I64)); // value
        }
        // log_config: (topic_ptr, policy_ptr, level) -> ()
        FfiSymbol::LogConfig => {
            sig.params.push(AbiParam::new(I64)); // topic_ptr (handle Text ou 0)
            sig.params.push(AbiParam::new(I64)); // policy_ptr (handle Text ou 0)
            sig.params.push(AbiParam::new(I64)); // level (tag do enum LogLevel)
            // sem returns — Unit
        }
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
        // ── Bytes / Byte (PRD-bytes) ──
        // bytes_alloc: (len, arena) -> ptr
        FfiSymbol::BytesAlloc => {
            sig.params.push(AbiParam::new(I64)); // len
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_from_ptr: (src, len, arena) -> ptr
        FfiSymbol::BytesFromPtr => {
            sig.params.push(AbiParam::new(I64)); // src
            sig.params.push(AbiParam::new(I64)); // len
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_from_ints: (ptrs, count, arena) -> ptr
        FfiSymbol::BytesFromInts => {
            sig.params.push(AbiParam::new(I64)); // ptrs
            sig.params.push(AbiParam::new(I64)); // count
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_len: (ptr) -> i64
        FfiSymbol::BytesLen => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // len
        }
        // bytes_get: (ptr, idx) -> i64
        FfiSymbol::BytesGet => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // bytes_set: (ptr, idx, val) -> void
        FfiSymbol::BytesSet => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.params.push(AbiParam::new(I64)); // val
        }
        // bytes_get_checked: (ptr, idx) -> i64 (Result box)
        FfiSymbol::BytesGetChecked => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // bytes_concat: (a, b, arena) -> ptr
        FfiSymbol::BytesConcat => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_eq: (a, b) -> i64 (0/1)
        FfiSymbol::BytesEq => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // bool
        }
        // bytes_neq: (a, b) -> i64 (0/1)
        FfiSymbol::BytesNeq => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // bool
        }
        // bytes_show: (ptr) -> *mut c_char
        FfiSymbol::BytesShow => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // c_str ptr
        }
        // bytes_slice: (ptr, start, end, arena) -> ptr
        FfiSymbol::BytesSlice => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // start
            sig.params.push(AbiParam::new(I64)); // end
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_and: (a, b, arena) -> ptr
        FfiSymbol::BytesAnd => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_or: (a, b, arena) -> ptr
        FfiSymbol::BytesOr => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_xor: (a, b, arena) -> ptr
        FfiSymbol::BytesXor => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_not: (ptr, arena) -> ptr
        FfiSymbol::BytesNot => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // byte_and: (a, b) -> i64
        FfiSymbol::ByteAnd => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_or: (a, b) -> i64
        FfiSymbol::ByteOr => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_xor: (a, b) -> i64
        FfiSymbol::ByteXor => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_not: (a) -> i64
        FfiSymbol::ByteNot => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_shr: (a, n) -> i64
        FfiSymbol::ByteShr => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // n
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_shl: (a, n) -> i64
        FfiSymbol::ByteShl => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // n
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_to_int: (b) -> i64
        FfiSymbol::ByteToInt => {
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // int_to_byte: (n) -> i64
        FfiSymbol::IntToByte => {
            sig.params.push(AbiParam::new(I64)); // n
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // int_to_bytes: (n, arena) -> ptr
        FfiSymbol::IntToBytes => {
            sig.params.push(AbiParam::new(I64)); // n
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // text_to_bytes: (text_ptr, arena) -> ptr
        FfiSymbol::TextToBytes => {
            sig.params.push(AbiParam::new(I64)); // text_ptr
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_to_text: (bytes_ptr) -> *mut c_char
        FfiSymbol::BytesToText => {
            sig.params.push(AbiParam::new(I64)); // bytes_ptr
            sig.returns.push(AbiParam::new(I64)); // c_str ptr
        }
        // text_at: (text_ptr, idx, arena) -> i64 (Result box)
        FfiSymbol::TextAt => {
            sig.params.push(AbiParam::new(I64)); // text_ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // text_len: (text_ptr) -> i64
        FfiSymbol::TextLen => {
            sig.params.push(AbiParam::new(I64)); // text_ptr
            sig.returns.push(AbiParam::new(I64)); // len
        }
        // text_slice: (text_ptr, start, end) -> *mut c_char
        FfiSymbol::TextSlice => {
            sig.params.push(AbiParam::new(I64)); // text_ptr
            sig.params.push(AbiParam::new(I64)); // start
            sig.params.push(AbiParam::new(I64)); // end
            sig.returns.push(AbiParam::new(I64)); // c_str ptr
        }
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
        // to_bytes: (value_ptr, type_id, arena) -> bytes_ptr
        FfiSymbol::ToBytes => {
            sig.params.push(AbiParam::new(I64)); // value_ptr
            sig.params.push(AbiParam::new(I64)); // type_id
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // bytes_ptr
        }
        // from_bytes: (bytes_ptr, arena) -> value_ptr
        FfiSymbol::FromBytes => {
            sig.params.push(AbiParam::new(I64)); // bytes_ptr
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // value_ptr
        }
        // spawn_process: (fn_ptr, args_ptr, arena) -> void (fire-and-forget)
        FfiSymbol::SpawnProcess => {
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // args_ptr
            sig.params.push(AbiParam::new(I64)); // arena
        }
        // ── File I/O ──
        // file_open: (path_ptr, mode_tag) -> i64 (Result box ARC)
        FfiSymbol::FileOpen => {
            sig.params.push(AbiParam::new(I64)); // path_ptr (Text)
            sig.params.push(AbiParam::new(I64)); // mode_tag (FileMode variant tag)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_read: (handle) -> i64 (Result box ARC)
        FfiSymbol::FileRead => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_read_chunk: (handle, n) -> i64 (Result box)
        FfiSymbol::FileReadChunk => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // n (SMI-tagged)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_readline: (handle) -> i64 (Result box ARC)
        FfiSymbol::FileReadline => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_write_text: (handle, data_ptr) -> i64 (Result box ARC)
        FfiSymbol::FileWriteText => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // data_ptr (Text — C string)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_write_bytes: (handle, data_ptr) -> i64 (Result box ARC)
        FfiSymbol::FileWriteBytes => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // data_ptr (Bytes — blob with len header)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_close: (handle) -> void
        FfiSymbol::FileClose => {
            sig.params.push(AbiParam::new(I64)); // handle
        }
        // ── Socket I/O ──
        // socket_open: (kind_box, mode_box) -> i64 (Result box)
        FfiSymbol::SocketOpen => {
            sig.params.push(AbiParam::new(I64)); // kind_box (SocketKind Sum box)
            sig.params.push(AbiParam::new(I64)); // mode_box (SocketMode Sum box)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_listen: (listener_handle) -> i64 (Result box)
        FfiSymbol::SocketListen => {
            sig.params.push(AbiParam::new(I64)); // listener_handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_read: (handle) -> i64 (Result box)
        FfiSymbol::SocketRead => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_read_chunk: (handle, n) -> i64 (Result box)
        FfiSymbol::SocketReadChunk => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // n (SMI-tagged)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_readline: (handle) -> i64 (Result box)
        FfiSymbol::SocketReadline => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_write_text: (handle, data_ptr) -> i64 (Result box)
        FfiSymbol::SocketWriteText => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // data_ptr (Text — C string)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_write_bytes: (handle, data_ptr) -> i64 (Result box)
        FfiSymbol::SocketWriteBytes => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // data_ptr (Bytes — blob)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_close: (handle) -> void
        FfiSymbol::SocketClose => {
            sig.params.push(AbiParam::new(I64)); // handle
        }
        // ── Reflection (fn-reflection) ──
        // register_fn_meta_table: (ptr, count) -> void
        FfiSymbol::FnMetaRegister => {
            sig.params.push(AbiParam::new(I64)); // ptr to table
            sig.params.push(AbiParam::new(I64)); // count
        }
        // fn_meta_lookup: (fn_ptr, field) -> i64
        FfiSymbol::FnMetaLookup => {
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // field (0=name, 1=arity, ...)
            sig.returns.push(AbiParam::new(I64)); // value
        }
    }

    sig
}

/// Retorna `true` se a FFI precisa de `arena_handle` injetado como último
/// argumento. Estas FFIs têm `arena` como último param na assinatura, mas
/// o caller (via `Closure` / `+` / `cons` / etc.) não fornece esse arg —
/// o codegen deve injetá-lo automaticamente.
///
/// Lista gerada a partir das assinaturas em `ffi_signature` onde o último
/// param é `arena` ou `arena_handle`. Se uma FFI nova for adicionada com
/// arena como último param, adicione-a aqui também.
pub(crate) fn ffi_needs_arena(sym_name: &str) -> bool {
    matches!(
        FfiSymbol::from_name(sym_name),
        Some(
            FfiSymbol::ArenaAlloc
                | FfiSymbol::ArenaDestroy
                | FfiSymbol::StoreSumResult
                | FfiSymbol::AllocArc
                | FfiSymbol::ListCons
                | FfiSymbol::ArrayAlloc
                | FfiSymbol::RangeAlloc
                | FfiSymbol::ListReverse
                | FfiSymbol::ListConcat
                | FfiSymbol::ChannelCreate
                | FfiSymbol::QueueCreate
                | FfiSymbol::BroadcastCreate
                | FfiSymbol::DictEmpty
                | FfiSymbol::DictInsert
                | FfiSymbol::DictGetChecked
                | FfiSymbol::DictRemove
                | FfiSymbol::DictNext
                | FfiSymbol::SetEmpty
                | FfiSymbol::SetInsert
                | FfiSymbol::SetRemove
                | FfiSymbol::SetNext
                | FfiSymbol::SetUnion
                | FfiSymbol::SetIntersection
                | FfiSymbol::SetDifference
                | FfiSymbol::DictMerge
                | FfiSymbol::BytesAlloc
                | FfiSymbol::BytesFromPtr
                | FfiSymbol::BytesFromInts
                | FfiSymbol::BytesConcat
                | FfiSymbol::BytesSlice
                | FfiSymbol::BytesAnd
                | FfiSymbol::BytesOr
                | FfiSymbol::BytesXor
                | FfiSymbol::BytesNot
                | FfiSymbol::IntToBytes
                | FfiSymbol::TextToBytes
                | FfiSymbol::TextAt
                | FfiSymbol::ArraySlice
                | FfiSymbol::ListSlice
        )
    )
}
