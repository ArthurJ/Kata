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
/// Function → I64 (function pointer — Fio 9+).
/// InferVar → I64 (fallback graceful — não deveria chegar aqui).
pub(crate) fn ty_to_clif(ty: &Ty) -> cranelift_codegen::ir::Type {
    match ty {
        Ty::Prim(PrimTy::Int) => I64,
        Ty::Prim(PrimTy::Float) => F64,
        Ty::Prim(PrimTy::Text) | Ty::Prim(PrimTy::Rational) => I64,
        Ty::Unit => I64, // Unit é representado como I64 zero
        Ty::Struct(_) | Ty::Sum(_) | Ty::Tuple(_) | Ty::List(_) | Ty::Array(_) | Ty::Range(_)
        | Ty::Sender(_) | Ty::Receiver(_) | Ty::ReceiverFactory(_) => {
            I64
        }
        Ty::Function(_, _) => I64,
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
        // ── Scheduler/Fiber (Fase 10) ──
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
        // ── Arc<T> / CaptureBox (Fase 12) ──
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
        // ── Collections (Fio 8 Fase 6) ──
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
    }

    sig
}
