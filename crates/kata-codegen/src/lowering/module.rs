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

use thiserror::Error;

/// Erro interno do codegen — bug ou limitação do compilador, nunca do usuário.
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum CodegenError {
    /// Construção da TAST não suportada neste lowering (limitação do compilador).
    #[error("construção não suportada no codegen: {node}\nisto é uma limitação do compilador, não um erro no seu código")]
    #[diagnostic(code = "codegen.unsupported", help = "abra uma issue descrevendo o que você estava tentando fazer")]
    UnsupportedNode { node: String },

    /// Erro interno do Cranelift (bug do compilador).
    #[error("erro interno do Cranelift: {reason}\nisto é um bug do compilador, não um erro no seu código")]
    #[diagnostic(code = "codegen.cranelift", help = "abra uma issue com o código que causou este erro")]
    Cranelift { reason: String },

    /// Símbolo FFI não encontrado no runtime.
    #[error("símbolo FFI não encontrado: {symbol}")]
    #[diagnostic(code = "codegen.ffi_not_found", help = "verifique se o runtime foi linkado corretamente")]
    FfiSymbolNotFound { symbol: String },
}

/// Tabela de strings literais — indexada por índice.
pub(crate) type StringTable = Vec<String>;

/// Chave composta para funções Kata: (nome, tipos de entrada, tipo de saída).
/// Substitui String para evitar colisão entre overloads do mesmo método.
/// `Ty` já implementa `Hash + Eq`.
pub(crate) type FuncKey = (String, Vec<Ty>, Ty);

/// Tabela de símbolos de funções Kata nomeadas — mapeia chave composta → FuncId.
pub(crate) type SymbolTable = HashMap<FuncKey, cranelift_module::FuncId>;

/// Info de uma função nomeada recém-compilada nesta invocação de `lower_module`.
/// O caller (REPL) usa isto para extrair function pointers e registrar na
/// próxima linha como `Linkage::Import`.
pub(crate) struct CompiledFunc {
    /// Hash canônico (FNV-1a de nome + param_types + clauses).
    pub fn_hash: i64,
    /// Nome do símbolo no JITModule (ex: `__kata_fn_0`).
    pub cranelift_name: String,
    /// FuncId no JITModule atual — para `get_finalized_function`.
    pub func_id: cranelift_module::FuncId,
}

/// Lower do `TypedModule` completo: cria a função `__kata_entry` e
/// retorna o `MetadataTable` sidecar, a string table, os wrappers de teste,
/// e info das funções nomeadas recém-compiladas (para REPL incremental).
pub(crate) fn lower_module(
    typed: &TypedModule,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    struct_registry: &kata_core::StructRegistry,
    type_id_map: &HashMap<Ty, i64>,
    prev_funcs: &HashMap<i64, (String, *const u8)>,
) -> Result<
    (
        MetadataTable,
        StringTable,
        Vec<super::test_runner::TestWrapper>,
        Vec<CompiledFunc>,
    ),
    CodegenError,
> {
    let mut metadata = MetadataTable::new();
    let mut string_table = StringTable::new();
    let mut bytes_table: Vec<Vec<u8>> = Vec::new();
    let mut symbol_table: SymbolTable = HashMap::new();
    let mut fn_counter = 0u64;

    // ── Declara e define funções nomeadas antes do entry point ──
    let mut func_ids: Vec<cranelift_module::FuncId> = Vec::new();
    let mut compiled_funcs: Vec<CompiledFunc> = Vec::new();
    for func in &typed.functions {
        let fn_hash = crate::lowering::cache_key::canonical_fn_id(
            &func.name,
            &func.param_types,
            &func.clauses,
        );
        if let Some((sym_name, _fn_ptr)) = prev_funcs.get(&fn_hash) {
            // Função já compilada em linha anterior — declarar como Import.
            // O símbolo foi registrado no JITBuilder pelo caller (jit_eval_repl).
            let func_id =
                declare_kata_function(func, sym_name, Linkage::Import, module, struct_registry)?;
            symbol_table.insert(
                (func.name.clone(), func.param_types.clone(), func.ret_ty.clone()),
                func_id,
            );
            func_ids.push(func_id);
        } else {
            // Função nova — declarar como Export e definir corpo.
            let cranelift_name = format!("__kata_fn_{fn_counter}");
            fn_counter += 1;
            let func_id = declare_kata_function(
                func,
                &cranelift_name,
                Linkage::Export,
                module,
                struct_registry,
            )?;
            symbol_table.insert(
                (func.name.clone(), func.param_types.clone(), func.ret_ty.clone()),
                func_id,
            );
            compiled_funcs.push(CompiledFunc {
                fn_hash,
                cranelift_name,
                func_id,
            });
            func_ids.push(func_id);
        }
    }

    // Definir corpo apenas das funções Export (novas).
    // Funções Import (prev_funcs) já têm código compilado — só precisam
    // do FuncId no symbol_table para resolver chamadas.
    for (i, func) in typed.functions.iter().enumerate() {
        let fn_hash = crate::lowering::cache_key::canonical_fn_id(
            &func.name,
            &func.param_types,
            &func.clauses,
        );
        if !prev_funcs.contains_key(&fn_hash) {
            define_kata_function(
                func,
                func_ids[i],
                module,
                ffi_ids,
                &symbol_table,
                &mut string_table,
                &mut bytes_table,
                struct_registry,
                type_id_map,
            )?;
        }
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
            &mut bytes_table,
            struct_registry,
            type_id_map,
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
        &mut bytes_table,
        &mut fn_counter,
        struct_registry,
        type_id_map,
    )?;

    // Determina o tipo de retorno do entry point.
    let ret_ty = &typed.entry.node.ty;
    let ret_clif = crate::ffi_sigs::ty_to_clif(ret_ty);

    // Assinatura do __kata_entry: (rt: i64) → ret_clif
    // A2: rt é ponteiro para Box<Runtime>, passado pelo driver.
    let mut sig = Signature::new(CallConv::SystemV);
    sig.params.push(AbiParam::new(I64)); // rt
    sig.returns.push(AbiParam::new(ret_clif));

    let entry_id = module
        .declare_function("__kata_entry", Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift { reason: format!("declare __kata_entry: {e}") })?;

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
            .map_err(|e| CodegenError::Cranelift { reason: format!("declare_data {bytes_sym}: {e}") })?;
        let rebase_id = module
            .declare_data(&rebase_sym, Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift { reason: format!("declare_data {rebase_sym}: {e}") })?;
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

        // A2: rt é o primeiro (e único) block param — ponteiro para Box<Runtime>.
        let rt_value = builder.block_params(entry_block)[0];

        let mut lower = LowerCtx {
            builder: &mut builder,
            module,
            ffi_refs: &ffi_refs,
            kata_refs: &kata_refs,
            ffi_ids,
            kata_ids: &symbol_table,
            metadata: &mut metadata,
            string_table: &mut string_table,
            bytes_table: &mut bytes_table,
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
            io_handle_vars: Vec::new(),
            struct_registry,
            type_id_map,
            ipc_broker_fid: None,
            rt: None,
        };

        // Prólogo do entry point: inicializa scheduler (cria arena raiz internamente).
        // A2: scheduler_init(rt) → root_arena. rt é o block param do entry point.
        let scheduler_init_ref = lower
            .ffi_refs
            .get("kata_rt_scheduler_init")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_scheduler_init".into(),
            })?;
        let init_inst = lower.builder.ins().call(scheduler_init_ref, &[rt_value]);
        let root_arena = lower.builder.inst_results(init_inst)[0];
        lower.caller_arena = Some(root_arena);
        lower.rt = Some(rt_value);

        // ── Carregar snapshots comptime na root_arena ──
        // Para cada snapshot, chama kata_rt_load_snapshot(root_arena, bytes_ptr,
        // bytes_len, rebase_offsets_ptr, rebase_count, snapshot_id).
        if !typed.snapshots.is_empty() {
            let load_snap_ref = lower
                .ffi_refs
                .get("kata_rt_load_snapshot")
                .copied()
                .ok_or_else(|| CodegenError::FfiSymbolNotFound { symbol: "kata_rt_load_snapshot".into() })?;
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
        .map_err(|e| CodegenError::Cranelift { reason: format!("define __kata_entry: {e}") })?;
    module.clear_context(&mut ctx);

    // Define os data symbols para strings literais.
    for (i, s) in string_table.iter().enumerate() {
        let sym = format!("__kata_str_{i}");
        let did = module
            .declare_data(&sym, Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift { reason: format!("declare_data {sym}: {e}") })?;
        let mut data_desc = cranelift_module::DataDescription::new();
        // Null-terminated C string: o runtime usa CStr::from_ptr.
        let bytes = format!("{s}\0").into_bytes();
        data_desc.define(bytes.into());
        module
            .define_data(did, &data_desc)
            .map_err(|e| CodegenError::Cranelift { reason: format!("define_data {sym}: {e}") })?;
    }

    // Define os data symbols para bytes literais.
    for (i, b) in bytes_table.iter().enumerate() {
        let sym = format!("__kata_bytes_{i}");
        let did = module
            .declare_data(&sym, Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift { reason: format!("declare_data {sym}: {e}") })?;
        let mut data_desc = cranelift_module::DataDescription::new();
        data_desc.define(b.clone().into());
        module
            .define_data(did, &data_desc)
            .map_err(|e| CodegenError::Cranelift { reason: format!("define_data {sym}: {e}") })?;
    }

    // Define os data symbols para snapshots comptime.
    for (i, snap) in typed.snapshots.iter().enumerate() {
        // Bytes do snapshot.
        let bytes_sym = format!("__kata_snap_bytes_{i}");
        let mut data_desc = cranelift_module::DataDescription::new();
        data_desc.define(snap.bytes.clone().into());
        module
            .define_data(snapshot_data_ids[i], &data_desc)
            .map_err(|e| CodegenError::Cranelift { reason: format!("define_data {bytes_sym}: {e}") })?;

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
            .map_err(|e| CodegenError::Cranelift { reason: format!("define_data {rebase_sym}: {e}") })?;
    }

    Ok((metadata, string_table, _test_wrappers, compiled_funcs))
}
