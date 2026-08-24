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
pub mod interface_registry;
pub mod refines_registry;
pub(crate) mod shape; // TypeShape/TypeId — zero consumidores cross-crate
pub mod snapshot;
pub mod struct_registry;
pub mod ty;
pub mod type_env;
pub mod type_graph;

pub use dispatch::{
    DispatchError, DispatchOutcome, DispatchTable, OverloadInfo, PartialDispatchResult, Score,
    match_score,
};
pub use enum_registry::{EnumRegistry, VariantInfo};
pub use escape::EscapeTarget;
pub use ffi::FfiSymbol;
pub use interface_registry::{
    ImplEntry, ImplMethodInfo, InterfaceInfo, InterfaceRegistry, InterfaceSignature,
};
pub use refines_registry::{RefinesEntry, RefinesRegistry};
pub use snapshot::HeapSnapshotData;
pub use struct_registry::{FieldInfo, StructInfo, StructKey, StructRegistry};
pub use ty::{PrimTy, Ty, TypeEnv};
pub use type_graph::{TypeEdge, TypeGraph, TypeGraphBuilder, TypeId, TypeKind, TypeNode};
