//! Catálogo de assinaturas FFI para Cranelift.
//!
//! Mapeia cada `FfiSymbol` para uma `Signature` Cranelift com `AbiParam`
//! tipados corretamente. O codegen usa isto para declarar imports no
//! `JITModule` e `FunctionBuilder`.
//!
//! O catálogo é dividido por categoria de FFI em submódulos:
//! - [`arithmetic`]: aritmética, comparação, conversões de tipos primitivos.
//! - [`io`]: I/O simples, panic, timer.
//! - [`arena`]: arena e sum (box tag+payload).
//! - [`scheduler`]: scheduler/fiber, arc/capturebox, spawn de processos.
//! - [`collections`]: list, array, range, hash, dict, set, slices.
//! - [`channels`]: canais CSP, queues, broadcast, IPC, select, logging.
//! - [`comptime`]: comptime snapshots e cache.
//! - [`bytes`]: bytes, byte, conversões bytes↔text, serialização.
//! - [`file_io`]: file I/O e socket I/O.

mod arena;
mod arithmetic;
mod bytes;
mod channels;
mod collections;
mod comptime;
mod file_io;
mod io;
mod scheduler;

use cranelift_codegen::ir::types::{F64, I64};
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
        // OverloadSet: tipo interno, não deveria chegar ao codegen como valor.
        // Mapeia para I64 como fallback graceful.
        Ty::OverloadSet { .. } => I64,
    }
}

/// Constrói a assinatura Cranelift para um `FfiSymbol`.
///
/// O dispatch é feito por categoria em submódulos. Cada submódulo cobre um
/// subconjunto disjunto de variantes de `FfiSymbol`. Se todas as categorias
/// retornarem `None`, o símbolo está sem assinatura — bug de cobertura.
///
/// Match exaustivo — se uma variante nova for adicionada ao enum
/// `FfiSymbol` sem assinatura aqui, o `unreachable!` abaixo dispara em
/// runtime (idealmente seria um erro de compilação, mas a divisão por
/// categoria impede um único match exaustivo).
pub(crate) fn ffi_signature(sym: FfiSymbol) -> cranelift_codegen::ir::Signature {
    arithmetic::sig_for(sym)
        .or_else(|| io::sig_for(sym))
        .or_else(|| arena::sig_for(sym))
        .or_else(|| scheduler::sig_for(sym))
        .or_else(|| collections::sig_for(sym))
        .or_else(|| channels::sig_for(sym))
        .or_else(|| comptime::sig_for(sym))
        .or_else(|| bytes::sig_for(sym))
        .or_else(|| file_io::sig_for(sym))
        .unwrap_or_else(|| {
            unreachable!(
                "FfiSymbol::{sym:?} sem assinatura FFI — adicione-a ao submódulo apropriado"
            )
        })
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
                | FfiSymbol::DictNextSmi
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
