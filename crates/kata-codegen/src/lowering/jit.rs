//! Pipeline JIT completo — compila e executa um `TypedModule`.

use std::collections::HashMap;

use cranelift_codegen::settings::Configurable;
use cranelift_module::Module;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::TypedModule;

use super::backend::{JitBackend, ModuleBackend};
use super::module::{CodegenError, lower_module};
use super::test_runner::TestWrapper;

/// Resultado da execução JIT — valor bruto + tipo canônico para display.
pub struct JitResult {
    /// Valor bruto retornado pela função JIT.
    /// Int: i64 SMI-taggeado. Float: f64 (reinterpretado). Text/Struct/Sum: ptr.
    pub raw: i64,
    /// Tipo canônico do entry point (para display).
    pub ty: Ty,
}

/// Compila e executa um `TypedModule` via Cranelift JIT.
///
/// Pipeline: criar JITBuilder → registrar símbolos FFI → declarar FFI →
/// lower_module → finalize → get_finalized_function →
/// transmutar → executar.
pub fn jit_eval(
    typed: &TypedModule,
    type_id_map: &HashMap<Ty, i64>,
    type_shapes: &[kata_rt::TypeShape],
) -> Result<JitResult, CodegenError> {
    // Configura preserve_frame_pointers = true (necessário para CallConv::Tail / return_call).
    let mut flags_builder = cranelift_codegen::settings::builder();
    flags_builder
        .set("preserve_frame_pointers", "true")
        .map_err(|e| CodegenError::Cranelift { reason: format!("set preserve_frame_pointers: {e}") })?;
    let flags = cranelift_codegen::settings::Flags::new(flags_builder);

    let isa_builder = cranelift_native::builder()
        .map_err(|e| CodegenError::Cranelift { reason: format!("native isa builder: {e}") })?;
    let isa = isa_builder
        .finish(flags)
        .map_err(|e| CodegenError::Cranelift { reason: format!("isa finish: {e}") })?;

    let mut builder =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

    crate::ffi_registry::register_ffi_symbols(&mut builder);

    let inner = cranelift_jit::JITModule::new(builder);
    let mut backend = JitBackend::new(inner);

    let ffi_ids = crate::ffi_registry::declare_ffi_symbols(&mut backend)?;

    // Declara __kata_entry e faz o lowering.
    let ret_ty = typed.entry.node.ty.clone();
    let (_metadata, _string_table, _test_wrappers) = lower_module(
        typed,
        &mut backend,
        &ffi_ids,
        &typed.struct_registry,
        type_id_map,
    )?;

    // Finaliza todas as definições — resolve relocations, compila machine code.
    backend.finalize()?;

    // Obtém o ponteiro da função entry.
    let entry_id = backend
        .get_name("__kata_entry")
        .ok_or_else(|| CodegenError::Cranelift { reason: "__kata_entry não encontrado".into() })?;
    let entry_fid = match entry_id {
        cranelift_module::FuncOrDataId::Func(fid) => fid,
        _ => return Err(CodegenError::Cranelift { reason: "__kata_entry não é função".into() }),
    };
    let code = backend.get_finalized_function(entry_fid);

    // A2: Aloca Runtime para a execução. O entry point recebe rt como param.
    let rt = Box::new(kata_rt::Runtime::new());
    let rt_ptr = Box::into_raw(rt) as i64;

    // A2: Registrar type_shapes no Runtime (marshalling to_bytes/from_bytes).
    if !type_shapes.is_empty() {
        kata_rt::register_type_table(rt_ptr, type_shapes.to_vec());
    }

    // Mantém o module vivo enquanto executamos — os ponteiros são válidos
    // apenas enquanto o JITModule existe.
    let result = match &ret_ty {
        Ty::Prim(PrimTy::Float) => {
            // Float: a função retorna f64, mas JIT usa calling convention
            // que pode retornar em registrador float. Transmutar para fn(i64) -> f64.
            // SAFETY: `code` é ponteiro válido de Cranelift após finalize_definitions.
            // A assinatura da função foi construída com return type F64.
            let func: extern "C" fn(i64) -> f64 = unsafe { std::mem::transmute(code) };
            let f = func(rt_ptr);
            // Reinterpretar f64 como i64 para JitResult.raw.
            JitResult {
                raw: f.to_bits() as i64,
                ty: ret_ty,
            }
        }
        _ => {
            // Int, Text, Struct, Sum, Unit: retorna i64.
            // SAFETY: `code` é ponteiro válido de Cranelift após finalize_definitions.
            // A assinatura da função foi construída com return type I64.
            let func: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(code) };
            let val = func(rt_ptr);
            JitResult {
                raw: val,
                ty: ret_ty,
            }
        }
    };

    // A2: Leak do Runtime para preservar arenas — o valor retornado pode
    // ser um ponteiro para a arena Bump. Droppar o Runtime libera a arena
    // e invalida o ponteiro. Em produção (driver), o Runtime vive enquanto
    // o processo. Em testes JIT (jit_eval), o leak é aceitável.
    // TODO: para REPL/LSP, o Runtime deve ser droppado explicitamente após
    // o valor ser consumido/copiado.
    std::mem::forget(unsafe { Box::from_raw(rt_ptr as *mut kata_rt::Runtime) });

    // O module precisa sobreviver até aqui — dropping após execução.
    // Cranelift JIT mantém as páginas de código mapeadas enquanto o module vive.
    // Como `module` é dropped no fim deste escopo, o código já executou.
    std::mem::forget(backend.into_inner());

    Ok(result)
}

/// Compila um `TypedModule` e retorna os wrappers de teste gerados.
///
/// Diferente de `jit_eval`, não executa o entry point — apenas compila
/// e retorna os wrappers `__kata_test_*` descobertos. O driver `kata test`
/// usa isto para descobrir e executar testes individualmente.
///
/// Mantém o `JITModule` vivo (retornado) para que os ponteiros dos
/// wrappers permaneçam válidos durante a execução dos testes.
pub fn jit_compile_tests(
    typed: &TypedModule,
    type_id_map: &HashMap<Ty, i64>,
) -> Result<(cranelift_jit::JITModule, Vec<TestWrapper>), CodegenError> {
    let mut flags_builder = cranelift_codegen::settings::builder();
    flags_builder
        .set("preserve_frame_pointers", "true")
        .map_err(|e| CodegenError::Cranelift { reason: format!("set preserve_frame_pointers: {e}") })?;
    let flags = cranelift_codegen::settings::Flags::new(flags_builder);

    let isa_builder = cranelift_native::builder()
        .map_err(|e| CodegenError::Cranelift { reason: format!("native isa builder: {e}") })?;
    let isa = isa_builder
        .finish(flags)
        .map_err(|e| CodegenError::Cranelift { reason: format!("isa finish: {e}") })?;

    let mut builder =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

    crate::ffi_registry::register_ffi_symbols(&mut builder);

    let inner = cranelift_jit::JITModule::new(builder);
    let mut backend = JitBackend::new(inner);

    let ffi_ids = crate::ffi_registry::declare_ffi_symbols(&mut backend)?;

    let (_metadata, _string_table, test_wrappers) = lower_module(
        typed,
        &mut backend,
        &ffi_ids,
        &typed.struct_registry,
        type_id_map,
    )?;

    backend.finalize()?;

    Ok((backend.into_inner(), test_wrappers))
}
