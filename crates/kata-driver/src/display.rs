//! Display de resultados de execução — ponte entre `Ty` do compilador
//! e `kata_rt_print_result` do runtime.
//!
//! A lógica de display (SMI untag, BigInt show, Float bits→f64, Text CStr,
//! Rational, Boolean, Unit) vive em `kata-rt/src/display.rs` — ponto único.
//! Este módulo converte `Ty` → `type_tag` (i32) e delega para o runtime.

use kata_core::ty::{PrimTy, Ty};
use kata_rt as rt;

/// Imprime o resultado da execução delegando para `kata_rt_print_result`.
///
/// `raw` é o valor bruto retornado por `__kata_entry`. `ty` é o tipo do
/// entry point (de `TypedModule.entry.node.ty`).
pub(crate) fn print_result(raw: i64, ty: &Ty) {
    let tag = ty_to_type_tag(ty);
    // SAFETY: `kata_rt_print_result` é `extern "C"` e lê `raw` + `tag`.
    // Para TYPE_TEXT/TYPE_RATIONAL, `raw` é ponteiro válido produzido
    // pelo codegen — o runtime faz dereference seguro.
    unsafe { rt::kata_rt_print_result(raw, tag) };
}

/// Converte `Ty` do entry point para o tag serializável do runtime.
fn ty_to_type_tag(ty: &Ty) -> i32 {
    match ty {
        Ty::Prim(PrimTy::Int) => rt::TYPE_INT,
        Ty::Prim(PrimTy::Float) => rt::TYPE_FLOAT,
        Ty::Prim(PrimTy::Text) => rt::TYPE_TEXT,
        Ty::Prim(PrimTy::Rational) => rt::TYPE_RATIONAL,
        Ty::Sum(name) if name == "Boolean" => rt::TYPE_BOOLEAN,
        Ty::Unit => rt::TYPE_UNIT,
        _ => rt::TYPE_OTHER,
    }
}
