//! Seleção de calling convention por plataforma.
//!
//! FFI usa a ABI nativa do target:
//! - Linux/macOS x86_64 → `SystemV`
//! - Windows x86_64     → `WindowsFastcall`
//! - macOS aarch64      → `SystemV` (Cranelift mapeia para AAPCS)
//!
//! `CallConv::Tail` (Actions e funções Kata) é suportado pelo Cranelift em
//! todas as plataformas-alvo; verificação empírica na Fase 6 do PRD-Windows.

use cranelift_codegen::isa::CallConv;

/// Returns the native FFI calling convention for the target platform.
pub(crate) fn ffi_call_conv() -> CallConv {
    #[cfg(target_os = "windows")]
    {
        CallConv::WindowsFastcall
    }
    #[cfg(not(target_os = "windows"))]
    {
        CallConv::SystemV
    }
}
