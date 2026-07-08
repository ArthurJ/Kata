//! Fundação transversal do compilador Kata.
//!
//! Define os contratos compartilhados entre todas as fases do pipeline:
//! - [`Ty`] — tipo canônico
//! - [`PrimTy`] — mapeamento de representação FFI (não tipo da linguagem)
//! - [`TypeEnv`] — árvore de escopos para name resolution
//! - [`FfiSymbol`] — enum tipado de símbolos FFI
//! - [`TypeShape`] — projeção runtime para reflexão estrutural
//! - [`TypeId`] — identificador u32 para a type table do runtime

pub mod ffi;
pub mod shape;
pub mod ty;

pub use ffi::FfiSymbol;
pub use shape::{TypeId, TypeShape};
pub use ty::{PrimTy, Ty, TypeEnv};
