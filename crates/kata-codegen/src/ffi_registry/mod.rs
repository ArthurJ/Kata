//! Registro e declaração de símbolos FFI no JIT.
//!
//! `register_ffi_symbols` popula o `JITBuilder` com os ponteiros das funções
//! C do `kata-rt`. `declare_ffi_symbols` declara os imports no `JITModule`
//! e retorna o mapa nome → FuncId.

mod declare;
mod register;
mod symbols;

pub(crate) use declare::declare_ffi_symbols;
pub(crate) use register::register_ffi_symbols;
