//! Lowering TAST → CLIF (Cranelift IR) + MetadataTable sidecar + emit.
//!
//! Sem IR intermediária própria — o lowering é direto TAST → CLIF.
//! Block arguments nativos (Cranelift 0.133) — sem stack slots.
//! MetadataTable é read-only após lowering, consultada pelo ARC pass.

pub mod ffi_sigs;
pub mod lowering;
pub mod metadata;

pub use lowering::{
    CodegenError, JitResult, declare_ffi_symbols, jit_eval, lower_module, register_ffi_symbols,
};
pub use metadata::MetadataTable;
