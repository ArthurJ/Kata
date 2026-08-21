//! Pipeline JIT completo — compila e executa um `TypedModule`.

use std::collections::HashMap;

use cranelift_codegen::settings::Configurable;
use cranelift_module::Module;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::TypedModule;

use super::backend::{JitBackend, ModuleBackend};
use super::module::{CodegenError, lower_module};
use super::test_runner::TestWrapper;

/// Cria um `Runtime` e faz leak (retorna o ponteiro bruto).
///
/// Para testes E2E short-lived: o Runtime precisa sobreviver à execução JIT
/// porque valores retornados (List, Struct, etc.) são ponteiros para a
/// arena Bump. O leak é aceitável em testes (processo efêmero).
///
/// Para REPL/LSP/driver: não usar este helper — criar `Box<Runtime>`
/// explicitamente e gerenciar o lifecycle.
pub fn leak_rt_ptr() -> i64 {
    let rt = Box::new(kata_rt::Runtime::new());
    Box::into_raw(rt) as i64
}

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
///
/// O caller é responsável por criar o `Runtime` e passar `rt_ptr`.
/// Para testes E2E short-lived, o caller pode fazer leak do Runtime
/// (valores retornados são ponteiros para a arena Bump). Para REPL/LSP,
/// o caller mantém o Runtime vivo entre avaliações para persistir estado.
pub fn jit_eval(
    typed: &TypedModule,
    type_id_map: &HashMap<Ty, i64>,
    type_shapes: &[kata_rt::TypeShape],
    rt_ptr: i64,
    emit_ir: bool,
) -> Result<JitResult, CodegenError> {
    // Configura preserve_frame_pointers = true (necessário para CallConv::Tail / return_call).
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

    let mut builder =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

    crate::ffi_registry::register_ffi_symbols(&mut builder);

    let inner = cranelift_jit::JITModule::new(builder);
    let mut backend = JitBackend::new(inner);

    let ffi_ids = crate::ffi_registry::declare_ffi_symbols(&mut backend)?;

    // Declara __kata_entry e faz o lowering.
    let ret_ty = typed.entry.node.ty.clone();
    let (_metadata, _string_table, _test_wrappers, _compiled_funcs, ir_dump) = lower_module(
        typed,
        &mut backend,
        &ffi_ids,
        &typed.struct_registry,
        type_id_map,
        &HashMap::new(),
        emit_ir,
    )?;

    // Imprimir CLIF no stderr se solicitado (antes de finalizar/executar).
    if emit_ir {
        for (name, clif) in &ir_dump {
            eprintln!(";; ── {name} ──");
            eprintln!("{clif}");
        }
    }

    // Finaliza todas as definições — resolve relocations, compila machine code.
    backend.finalize()?;

    // Obtém o ponteiro da função entry.
    let entry_id = backend
        .get_name("__kata_entry")
        .ok_or_else(|| CodegenError::Cranelift {
            reason: "__kata_entry não encontrado".into(),
        })?;
    let entry_fid = match entry_id {
        cranelift_module::FuncOrDataId::Func(fid) => fid,
        _ => {
            return Err(CodegenError::Cranelift {
                reason: "__kata_entry não é função".into(),
            });
        }
    };
    let code = backend.get_finalized_function(entry_fid);

    // Registrar type_shapes no Runtime (marshalling to_bytes/from_bytes).
    // O caller é responsável pelo lifecycle do Runtime.
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

    let mut builder =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

    crate::ffi_registry::register_ffi_symbols(&mut builder);

    let inner = cranelift_jit::JITModule::new(builder);
    let mut backend = JitBackend::new(inner);

    let ffi_ids = crate::ffi_registry::declare_ffi_symbols(&mut backend)?;

    let (_metadata, _string_table, test_wrappers, _compiled_funcs, _ir_dump) = lower_module(
        typed,
        &mut backend,
        &ffi_ids,
        &typed.struct_registry,
        type_id_map,
        &HashMap::new(),
        false,
    )?;

    backend.finalize()?;

    Ok((backend.into_inner(), test_wrappers))
}

/// Função nomeada persistida entre linhas do REPL.
/// Mapeia hash → (nome do símbolo JIT, ponteiro de código absoluto).
/// O ponteiro permanece válido porque o JITModule anterior é leaked
/// (páginas de código permanecem mapeadas).
pub type PrevFuncMap = HashMap<i64, (String, *const u8)>;

/// Resultado de `jit_eval_repl` — inclui function pointers das funções
/// nomeadas recém-compiladas nesta linha, para persistir na próxima.
pub struct ReplJitResult {
    /// Resultado da execução do entry point.
    pub jit: JitResult,
    /// Funções nomeadas recém-compiladas: (fn_hash, cranelift_name, fn_ptr).
    /// O caller armazena em `function_table` para registrar como Import
    /// na próxima linha.
    pub new_funcs: Vec<(i64, String, *const u8)>,
}

/// Compila e executa um `TypedModule` via Cranelift JIT, com suporte a
/// funções nomeadas persistidas entre linhas do REPL.
///
/// Diferença vs `jit_eval`:
/// - `prev_funcs`: mapeia fn_hash → nome do símbolo JIT das funções já
///   compiladas em linhas anteriores. Estas são registradas no
///   `JITBuilder::symbol()` e declaradas como `Linkage::Import` pelo
///   `lower_module`. O corpo não é recompilado.
/// - Retorna `new_funcs`: function pointers das funções recém-compiladas
///   (Export) nesta linha. O caller armazena para usar como Import na
///   próxima linha.
///
/// O caller é responsável pelo lifecycle do Runtime (`rt_ptr`).
pub fn jit_eval_repl(
    typed: &TypedModule,
    type_id_map: &HashMap<Ty, i64>,
    type_shapes: &[kata_rt::TypeShape],
    rt_ptr: i64,
    prev_funcs: &PrevFuncMap,
) -> Result<ReplJitResult, CodegenError> {
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

    let mut builder =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

    // Registrar símbolos FFI.
    crate::ffi_registry::register_ffi_symbols(&mut builder);

    // Registrar function pointers das funções persistidas de linhas anteriores.
    // O nome do símbolo é o `cranelift_name` da linha original — o mesmo nome
    // que `lower_module` usa para declarar como `Linkage::Import`.
    for (sym_name, fn_ptr) in prev_funcs.values() {
        builder.symbol(sym_name, *fn_ptr);
    }

    let inner = cranelift_jit::JITModule::new(builder);
    let mut backend = JitBackend::new(inner);

    let ffi_ids = crate::ffi_registry::declare_ffi_symbols(&mut backend)?;

    let ret_ty = typed.entry.node.ty.clone();
    let (_metadata, _string_table, _test_wrappers, compiled_funcs, _ir_dump) = lower_module(
        typed,
        &mut backend,
        &ffi_ids,
        &typed.struct_registry,
        type_id_map,
        prev_funcs,
        false,
    )?;

    // Finaliza todas as definições.
    backend.finalize()?;

    // Obtém o ponteiro da função entry.
    let entry_id = backend
        .get_name("__kata_entry")
        .ok_or_else(|| CodegenError::Cranelift {
            reason: "__kata_entry não encontrado".into(),
        })?;
    let entry_fid = match entry_id {
        cranelift_module::FuncOrDataId::Func(fid) => fid,
        _ => {
            return Err(CodegenError::Cranelift {
                reason: "__kata_entry não é função".into(),
            });
        }
    };
    let code = backend.get_finalized_function(entry_fid);

    // Registrar type_shapes no Runtime se necessário.
    if !type_shapes.is_empty() {
        kata_rt::register_type_table(rt_ptr, type_shapes.to_vec());
    }

    // Executa o entry point.
    let jit = match &ret_ty {
        Ty::Prim(PrimTy::Float) => {
            let func: extern "C" fn(i64) -> f64 = unsafe { std::mem::transmute(code) };
            let f = func(rt_ptr);
            JitResult {
                raw: f.to_bits() as i64,
                ty: ret_ty,
            }
        }
        _ => {
            let func: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(code) };
            let val = func(rt_ptr);
            JitResult {
                raw: val,
                ty: ret_ty,
            }
        }
    };

    // Extrair function pointers das funções recém-compiladas (Export).
    let mut new_funcs = Vec::new();
    for cf in compiled_funcs {
        let ptr = backend.get_finalized_function(cf.func_id);
        new_funcs.push((cf.fn_hash, cf.cranelift_name, ptr));
    }

    // Leak do JITModule — páginas de código permanecem mapeadas.
    // Os function pointers extraídos acima permanecem válidos.
    std::mem::forget(backend.into_inner());

    Ok(ReplJitResult { jit, new_funcs })
}
