//! Fundação transversal do compilador Kata.
//!
//! Define os contratos compartilhados entre todas as fases do pipeline:
//! - [`Ty`] — tipo canônico
//! - [`PrimTy`] — mapeamento de representação FFI (não tipo da linguagem)
//! - [`TypeEnv`] — árvore de escopos para name resolution
//! - [`FfiSymbol`] — enum tipado de símbolos FFI
//! - [`TypeShape`] — projeção runtime para reflexão estrutural
//! - [`TypeId`] — identificador u32 para a type table do runtime
//! - [`DispatchTable`] — tabela de overloads com despacho por dominância
//! - [`Score`] — score 4D (exact, alias, refined, iface) para seleção

pub mod dispatch;
pub mod enum_registry;
pub mod escape;
pub mod ffi;
pub(crate) mod shape; // TypeShape/TypeId — zero consumidores cross-crate
pub mod ty;

pub use dispatch::{DispatchError, DispatchTable, OverloadInfo, PartialDispatchResult, Score};
pub use enum_registry::{EnumRegistry, VariantInfo};
pub use escape::EscapeTarget;
pub use ffi::FfiSymbol;
pub use ty::{PrimTy, Ty, TypeEnv};
