//! Lower do `TypedModule` completo: cria a função `__kata_entry` e
//! retorna o `MetadataTable` sidecar + a string table.
//!
//! Também orquestra declaração/definição de funções Kata nomeadas e Actions,
//! delegando para `function_def` e `action_def`.

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::TypedModule;

use super::LowerCtx;
use super::action_def::{declare_kata_action, define_kata_action};
use super::expr::lower_expr;
use super::function_def::{declare_kata_function, define_kata_function};
use crate::metadata::MetadataTable;

use super::backend::ModuleBackend;

/// Erro de codegen.
#[derive(Debug)]
pub enum CodegenError {
    /// Símbolo FFI não encontrado no runtime.
    FfiSymbolNotFound(String),
    /// Erro interno do Cranelift.
    Cranelift(String),
    /// Nó da TAST não suportado neste lowering.
    UnsupportedNode(String),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::FfiSymbolNotFound(s) => write!(f, "símbolo FFI não encontrado: {s}"),
            CodegenError::Cranelift(s) => write!(f, "erro Cranelift: {s}"),
            CodegenError::UnsupportedNode(s) => write!(f, "nó TAST não suportado: {s}"),
        }
    }
}

impl std::error::Error for CodegenError {}

/// Tabela de strings literais — indexada por índice.
pub(crate) type StringTable = Vec<String>;

/// Chave composta para funções Kata: (nome, tipos de entrada, tipo de saída).
/// Substitui String para evitar colisão entre overloads do mesmo método.
/// `Ty` já implementa `Hash + Eq`.
pub(crate) type FuncKey = (String, Vec<Ty>, Ty);

/// Tabela de símbolos de funções Kata nomeadas — mapeia chave composta → FuncId.
pub(crate) type SymbolTable = HashMap<FuncKey, cranelift_module::FuncId>;

/// Lower do `TypedModule` completo: cria a função `__kata_entry` e
/// retorna o `MetadataTable` sidecar, a string table, e os wrappers de teste.
pub(crate) fn lower_module(
    typed: &TypedModule,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    struct_registry: &kata_core::StructRegistry,
) -> Result<
    (
        MetadataTable,
        StringTable,
        Vec<super::test_runner::TestWrapper>,
    ),
    CodegenError,
> {
    let mut metadata = MetadataTable::new();
    let mut string_table = StringTable::new();
    let mut symbol_table: SymbolTable = HashMap::new();
    let mut fn_counter = 0u64;

    // ── Declara e define funções nomeadas antes do entry point ──
    let mut func_ids: Vec<cranelift_module::FuncId> = Vec::new();
    for func in &typed.functions {
        let cranelift_name = format!("__kata_fn_{fn_counter}");
        fn_counter += 1;
        let func_id = declare_kata_function(func, &cranelift_name, module, struct_registry)?;
        symbol_table.insert(
            (
                func.name.clone(),
                func.param_types.clone(),
                func.ret_ty.clone(),
            ),
            func_id,
        );
        func_ids.push(func_id);
    }

    for (i, func) in typed.functions.iter().enumerate() {
        define_kata_function(
            func,
            func_ids[i],
            module,
            ffi_ids,
            &symbol_table,
            &mut string_table,
            struct_registry,
        )?;
    }

    // ── Declara e define Actions antes do entry point ──
    let mut action_ids: Vec<cranelift_module::FuncId> = Vec::new();
    for action in &typed.actions {
        let cranelift_name = format!("__kata_fn_{fn_counter}");
        fn_counter += 1;
        let func_id = declare_kata_action(action, &cranelift_name, module)?;
        symbol_table.insert(
            (
                action.name.clone(),
                action.param_types.clone(),
                action.ret_ty.clone(),
            ),
            func_id,
        );
        action_ids.push(func_id);
    }

    for (i, action) in typed.actions.iter().enumerate() {
        define_kata_action(
            action,
            action_ids[i],
            module,
            ffi_ids,
            &symbol_table,
            &mut string_table,
            struct_registry,
        )?;
    }

    // ── Gera wrappers de teste `__kata_test_*` para cada @test ──
    // Após Actions declaradas/definidas (symbol_table populado com FuncIds).
    let _test_wrappers = super::test_runner::generate_test_wrappers(
        typed,
        module,
        ffi_ids,
        &symbol_table,
        &mut string_table,
        &mut fn_counter,
        struct_registry,
    )?;

    // Determina o tipo de retorno do entry point.
    let ret_ty = &typed.entry.node.ty;
    let ret_clif = crate::ffi_sigs::ty_to_clif(ret_ty);

    // Assinatura do __kata_entry: () → ret_clif
    let mut sig = Signature::new(CallConv::SystemV);
    sig.returns.push(AbiParam::new(ret_clif));

    let entry_id = module
        .declare_function("__kata_entry", Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare __kata_entry: {e}")))?;

    // Cria um Context do Cranelift (não FunctionBuilderContext).
    let mut ctx = module.make_context();

    // ── Declarar data symbols para snapshots comptime ──
    // Cada snapshot precisa de dois data symbols: bytes e rebase_offsets.
    // São declarados antes do entry point para serem referenciados no prólogo,
    // e definidos após define_function.
    let snapshot_count = typed.snapshots.len();
    let mut snapshot_data_ids: Vec<cranelift_module::DataId> = Vec::new();
    let mut snapshot_rebase_ids: Vec<cranelift_module::DataId> = Vec::new();
    for i in 0..snapshot_count {
        let bytes_sym = format!("__kata_snap_bytes_{i}");
        let rebase_sym = format!("__kata_snap_rebase_{i}");
        let bytes_id = module
            .declare_data(&bytes_sym, Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift(format!("declare_data {bytes_sym}: {e}")))?;
        let rebase_id = module
            .declare_data(&rebase_sym, Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift(format!("declare_data {rebase_sym}: {e}")))?;
        snapshot_data_ids.push(bytes_id);
        snapshot_rebase_ids.push(rebase_id);
    }

    // Constrói a função IR dentro do Context.
    {
        let func = &mut ctx.func;
        func.signature = sig.clone();

        // Declara cada FFI no Function e coleta os FuncRefs.
        let mut ffi_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (name, &fid) in ffi_ids {
            let func_ref = module.declare_func_in_func(fid, func);
            ffi_refs.insert(name.clone(), func_ref);
        }

        // Declara funções Kata nomeadas no Function (para call direto).
        let mut kata_refs: HashMap<FuncKey, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (key, &fid) in &symbol_table {
            let func_ref = module.declare_func_in_func(fid, func);
            kata_refs.insert(key.clone(), func_ref);
        }

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(func, &mut func_ctx);

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let mut lower = LowerCtx {
            builder: &mut builder,
            module,
            ffi_refs: &ffi_refs,
            kata_refs: &kata_refs,
            ffi_ids,
            kata_ids: &symbol_table,
            metadata: &mut metadata,
            string_table: &mut string_table,
            var_map: HashMap::new(),
            anon_counter: 0,
            emitted_tail_call: false,
            no_tail_calls: true, // entry point usa SystemV — sem return_call
            epilogue_block: None,
            fiber_arena: None,
            caller_arena: None,
            scheduler_mode: true, // entry point: ActionCalls via spawn+run
            loop_break_block: None,
            loop_continue_block: None,
            arc_vars: Vec::new(),
            struct_registry,
        };

        // Prólogo do entry point: inicializa scheduler (cria arena raiz internamente).
        // scheduler_init retorna o handle da arena raiz — usar como caller_arena.
        // Pré-11: substitui a antiga arena global (handle 0, nunca destruída).
        let scheduler_init_ref = lower
            .ffi_refs
            .get("kata_rt_scheduler_init")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_scheduler_init".into()))?;
        let init_inst = lower.builder.ins().call(scheduler_init_ref, &[]);
        let root_arena = lower.builder.inst_results(init_inst)[0];
        lower.caller_arena = Some(root_arena);

        // ── Carregar snapshots comptime na root_arena ──
        // Para cada snapshot, chama kata_rt_load_snapshot(root_arena, bytes_ptr,
        // bytes_len, rebase_offsets_ptr, rebase_count, snapshot_id).
        if !typed.snapshots.is_empty() {
            let load_snap_ref = lower
                .ffi_refs
                .get("kata_rt_load_snapshot")
                .copied()
                .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_load_snapshot".into()))?;
            let ptr_ty = lower.module.target_config().pointer_type();
            for (i, snap) in typed.snapshots.iter().enumerate() {
                // Obter GlobalValues para os data symbols.
                let bytes_gv = lower
                    .module
                    .declare_data_in_func(snapshot_data_ids[i], lower.builder.func);
                let bytes_ptr = lower.builder.ins().global_value(ptr_ty, bytes_gv);
                let rebase_gv = lower
                    .module
                    .declare_data_in_func(snapshot_rebase_ids[i], lower.builder.func);
                let rebase_ptr = lower.builder.ins().global_value(ptr_ty, rebase_gv);

                let bytes_len = lower.builder.ins().iconst(I64, snap.bytes.len() as i64);
                let rebase_count = lower
                    .builder
                    .ins()
                    .iconst(I64, snap.rebase_offsets.len() as i64);
                let snapshot_id = lower.builder.ins().iconst(I64, i as i64);

                lower.builder.ins().call(
                    load_snap_ref,
                    &[
                        root_arena,
                        bytes_ptr,
                        bytes_len,
                        rebase_ptr,
                        rebase_count,
                        snapshot_id,
                    ],
                );
            }
        }

        // Lowera pre_entry (let bindings e outras expressões top-level anteriores).
        // Estas são loweradas em sequência, compartilhando o var_map —
        // um `let` define uma variável que o entry pode usar.
        for pre in &typed.pre_entry {
            lower_expr(&pre.node, &mut lower)?;
        }

        let result = lower_expr(&typed.entry.node, &mut lower)?;

        // Bitcast F64→I64 na borda de retorno do entry point.
        // Necessário para refined types de Float: `17.5::Positivo` lowera
        // como F64 mas a assinatura do __kata_entry é I64 (Ty::Struct → I64).
        // Só bitcastar quando ret_clif é I64 e o valor é F64.
        let result = {
            let actual_ty = lower.builder.func.dfg.value_type(result);
            if actual_ty == cranelift_codegen::ir::types::F64 && ret_clif == I64 {
                lower
                    .builder
                    .ins()
                    .bitcast(I64, cranelift_codegen::ir::MemFlagsData::new(), result)
            } else {
                result
            }
        };
        lower.builder.ins().return_(&[result]);

        builder.finalize();
    }

    // Define a função no module usando o Context.
    module
        .define_function(entry_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift(format!("define __kata_entry: {e}")))?;
    module.clear_context(&mut ctx);

    // Define os data symbols para strings literais.
    for (i, s) in string_table.iter().enumerate() {
        let sym = format!("__kata_str_{i}");
        let did = module
            .declare_data(&sym, Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift(format!("declare_data {sym}: {e}")))?;
        let mut data_desc = cranelift_module::DataDescription::new();
        // Null-terminated C string: o runtime usa CStr::from_ptr.
        let bytes = format!("{s}\0").into_bytes();
        data_desc.define(bytes.into());
        module
            .define_data(did, &data_desc)
            .map_err(|e| CodegenError::Cranelift(format!("define_data {sym}: {e}")))?;
    }

    // Define os data symbols para snapshots comptime.
    for (i, snap) in typed.snapshots.iter().enumerate() {
        // Bytes do snapshot.
        let bytes_sym = format!("__kata_snap_bytes_{i}");
        let mut data_desc = cranelift_module::DataDescription::new();
        data_desc.define(snap.bytes.clone().into());
        module
            .define_data(snapshot_data_ids[i], &data_desc)
            .map_err(|e| CodegenError::Cranelift(format!("define_data {bytes_sym}: {e}")))?;

        // rebase_offsets — array de i64 (usize → i64 para ABI).
        let rebase_sym = format!("__kata_snap_rebase_{i}");
        let rebase_bytes: Vec<u8> = snap
            .rebase_offsets
            .iter()
            .flat_map(|off| (*off as i64).to_le_bytes())
            .collect();
        let mut rebase_desc = cranelift_module::DataDescription::new();
        rebase_desc.define(rebase_bytes.into());
        module
            .define_data(snapshot_rebase_ids[i], &rebase_desc)
            .map_err(|e| CodegenError::Cranelift(format!("define_data {rebase_sym}: {e}")))?;
    }

    Ok((metadata, string_table, _test_wrappers))
}
