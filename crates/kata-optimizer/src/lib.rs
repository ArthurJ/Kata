//! Passes de otimização.
//!
//! Constant folding, DCE, inlining e TCO são delegados ao Cranelift — o
//! JITModule já aplica otimizações nativas durante a compilação CLIF →
//! machine code.
//!
//! A função [`optimize`] existe para manter o pipeline simétrico — o driver
//! chama `optimize(&typed_module)` entre inference e codegen.
//!
//! Passes implementados:
//! - Fase 16: TRMA (Tail Recursion Modulo Associativity)

mod trma;

use kata_monomorph::MonoModule;

/// Otimiza o `MonoModule`.
///
/// Passes aplicados (em ordem):
/// 1. TRMA — detecta auto-recursão direta com operador associativo e
///    reescreve em recursão de cauda com acumulador.
pub fn optimize(mut mono: MonoModule) -> MonoModule {
    trma::trma_pass(&mut mono);
    mono
}
