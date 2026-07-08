//! Passes de otimização.
//!
//! No Fio 1: stub (pass-through). TRMA, StreamFusion e ARC pass vêm em fios
//! posteriores. Constant folding, DCE, inlining e TCO são delegados ao
//! Cranelift — o JITModule já aplica otimizações nativas durante a
//! compilação CLIF → machine code.
//!
//! A função [`optimize`] existe para manter o pipeline simétrico — o driver
//! chama `optimize(&typed_module)` entre inference e codegen. Em Fio 1,
//! retorna o `TypedModule` inalterado.

use kata_inference::TypedModule;

/// Otimiza o `TypedModule`. Em Fio 1, é pass-through (no-op).
///
/// Fios posteriores adicionam passes aqui:
/// - Fio 11: TRMA, escape analysis, tree shaking
/// - Fio 12: comptime pass (JIT-and-execute)
/// - Fio 9: ARC liveness pass
pub fn optimize(typed: TypedModule) -> TypedModule {
    // Pass-through — Cranelift faz as otimizações nativas.
    typed
}
