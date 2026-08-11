//! Pipeline AOT completo — emite um object file (`.o`) a partir de um `TypedModule`.
//!
//! Análogo a [`super::jit`] no que diz respeito ao lowering: cria um
//! [`AotBackend`] wrap de `cranelift_object::ObjectModule`, declara os
//! imports FFI (resolvidos em link-time, não há registro de ponteiros),
//! chama `lower_module`, `finalize()`, e então [`AotBackend::emit`] para
//! obter os bytes do object file. O linker do sistema resolve
//! os imports FFI contra `libkata_rt.a`/`.so`.

use std::collections::HashMap;

use cranelift_codegen::settings::Configurable;
use kata_core::ty::Ty;
use kata_inference::TypedModule;

use super::backend::{AotBackend, ModuleBackend};
use super::module::{CodegenError, lower_module};

/// Emite um object file (`.o`) a partir de um `TypedModule`.
///
/// Pipeline: configurar flags (mesmas do JIT) → criar `ObjectBuilder` →
/// `ObjectModule` → `AotBackend` → declarar FFI imports → `lower_module` →
/// `finalize` → `emit`. Os bytes retornados são um object file no formato
/// nativo do host (ELF/Mach-O/COFF) com relocations pendentes — o linker
/// resolve contra `libkata_rt`.
///
/// Diferença vs JIT: FFI são `Linkage::Import` sem registro de ponteiros
/// no builder. O `ObjectBuilder` não tem API de `symbol()` — os imports
/// são resolvidos em link-time pelo linker do sistema.
pub fn aot_emit(
    typed: &TypedModule,
    type_id_map: &HashMap<Ty, i64>,
) -> Result<Vec<u8>, CodegenError> {
    // Mesmas flags do JIT — preserve_frame_pointers é necessário para
    // CallConv::Tail / return_call (usado em tail calls de Actions).
    let mut flags_builder = cranelift_codegen::settings::builder();
    flags_builder
        .set("preserve_frame_pointers", "true")
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("set preserve_frame_pointers: {e}"),
        })?;
    let flags = cranelift_codegen::settings::Flags::new(flags_builder);

    let isa_builder = cranelift_native::builder().map_err(|e| CodegenError::Cranelift {
        reason: format!("native isa builder: {e}"),
    })?;
    let isa = isa_builder
        .finish(flags)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("isa finish: {e}"),
        })?;

    // ObjectBuilder precisa de um nome de arquivo (vai no símbolo de file).
    // O nome não afeta o resultado do link — é apenas metadata do object.
    let builder = cranelift_object::ObjectBuilder::new(
        isa,
        "kata_module",
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| CodegenError::Cranelift {
        reason: format!("ObjectBuilder::new: {e}"),
    })?;
    let inner = cranelift_object::ObjectModule::new(builder);
    let mut backend = AotBackend::new(inner);

    // FFI: declare_ffi_symbols declara imports (Linkage::Import). O linker
    // resolve contra libkata_rt — não há registro de ponteiros como no JIT.
    let ffi_ids = crate::ffi_registry::declare_ffi_symbols(&mut backend)?;

    // Lowering — reusa 100% do pipeline do JIT via &mut dyn ModuleBackend.
    let (_metadata, _string_table, _test_wrappers, _compiled_funcs) = lower_module(
        typed,
        &mut backend,
        &ffi_ids,
        &typed.struct_registry,
        type_id_map,
        &HashMap::new(),
    )?;

    // Finaliza: ObjectModule::finish() consome o module e produz ObjectProduct.
    backend.finalize()?;

    // Emite os bytes do object file.
    let object_bytes = backend.emit()?;

    Ok(object_bytes)
}
