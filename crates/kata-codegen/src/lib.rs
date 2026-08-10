//! Lowering TAST → CLIF (Cranelift IR) + MetadataTable sidecar + emit.
//!
//! Sem IR intermediária própria — o lowering é direto TAST → CLIF.
//! Block arguments nativos (Cranelift 0.133) — sem stack slots.
//! MetadataTable é read-only após lowering, consultada pelo ARC pass.

pub(crate) mod ffi_registry;
pub(crate) mod ffi_sigs;
pub(crate) mod lowering;
pub(crate) mod metadata;
pub(crate) mod smi;
pub mod type_table;

pub use lowering::{CodegenError, JitResult, TestWrapper, aot_emit, jit_compile_tests, jit_eval, leak_rt_ptr};
