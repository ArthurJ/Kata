//! Declaração de símbolos FFI no `JITModule`.

use crate::call_conv::ffi_call_conv;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_module::Linkage;

use crate::ffi_sigs::ffi_signature;
use crate::lowering::CodegenError;
use crate::lowering::ModuleBackend;

use super::symbols::all_ffi_symbols;

/// Declara todos os símbolos FFI no module e retorna o mapa nome → FuncId.
///
/// Usa `BTreeMap` (ordem determinística) em vez de `HashMap` para que os
/// `FuncRef`s sejam atribuídos na mesma ordem em todo run — a ordem do
/// `HashMap` é randomizada por processo, o que tornava o IR non-determinístico
/// e causava falhas intermitentes do Cranelift verifier.
pub(crate) fn declare_ffi_symbols(
    module: &mut dyn ModuleBackend,
) -> Result<std::collections::BTreeMap<String, cranelift_module::FuncId>, CodegenError> {
    let mut ffi_ids = std::collections::BTreeMap::new();
    for sym in all_ffi_symbols() {
        let name = sym.symbol_name();
        let sig = ffi_signature(sym);
        let fid = module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| CodegenError::Cranelift {
                reason: format!("declare FFI {name}: {e}"),
            })?;
        ffi_ids.insert(name.to_string(), fid);
    }
    // Símbolo especial: kata_rt_tag_int_from_str (não está no FfiSymbol enum).
    // Usado para lowerar IntLit que não cabe em SMI (BigInts).
    let tag_str_sig = {
        let mut sig = Signature::new(ffi_call_conv());
        sig.params.push(AbiParam::new(I64)); // ptr
        sig.params.push(AbiParam::new(I64)); // len
        sig.returns.push(AbiParam::new(I64)); // tagged i64
        sig
    };
    let tag_str_fid = module
        .declare_function("kata_rt_tag_int_from_str", Linkage::Import, &tag_str_sig)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("declare kata_rt_tag_int_from_str: {e}"),
        })?;
    ffi_ids.insert("kata_rt_tag_int_from_str".to_string(), tag_str_fid);

    // Símbolo especial: kata_rt_overflow_panic (não está no FfiSymbol enum).
    // Diverge (!) — o codegen emite trap(user(1)) após a call.
    // Assinatura: (rt: i64) -> void (Cranelift não tem tipo never; o trap
    // satisfaz o verificador).
    let overflow_panic_sig = {
        let mut sig = Signature::new(ffi_call_conv());
        sig.params.push(AbiParam::new(I64)); // rt
        sig
    };
    let overflow_panic_fid = module
        .declare_function(
            "kata_rt_overflow_panic",
            Linkage::Import,
            &overflow_panic_sig,
        )
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("declare kata_rt_overflow_panic: {e}"),
        })?;
    ffi_ids.insert("kata_rt_overflow_panic".to_string(), overflow_panic_fid);

    Ok(ffi_ids)
}
