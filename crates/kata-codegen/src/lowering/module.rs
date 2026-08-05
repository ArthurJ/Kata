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
    type_id_map: &HashMap<Ty, i64>,
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
    let mut bytes_table: Vec<Vec<u8>> = Vec::new();
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
            &mut bytes_table,
            struct_registry,
            type_id_map,
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

    // Declarar a sidecar table de reflexão (se houver funções).
    let fn_meta_table_did: Option<cranelift_module::DataId> = if !typed.functions.is_empty() {
        Some(
            module
                .declare_data("__kata_fn_meta_table", Linkage::Local, false, false)
                .map_err(|e| CodegenError::Cranelift(format!("declare_data fn_meta_table: {e}")))?,
        )
    } else {
        None
    };

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

        // ── Registrar sidecar table de reflexão ──
        // Carrega __kata_fn_meta_table e chama kata_rt_register_fn_meta_table(ptr, count).
        if let Some(table_did) = fn_meta_table_did {
            let register_ref = lower
                .ffi_refs
                .get("kata_rt_register_fn_meta_table")
                .copied()
                .ok_or_else(|| {
                    CodegenError::FfiSymbolNotFound("kata_rt_register_fn_meta_table".into())
                })?;
            let ptr_ty = lower.module.target_config().pointer_type();
            let table_gv = lower
                .module
                .declare_data_in_func(table_did, lower.builder.func);
            let table_ptr = lower.builder.ins().global_value(ptr_ty, table_gv);
            // count é o primeiro i64 da tabela (header)
            let count_val = lower.builder.ins().load(
                I64,
                cranelift_codegen::ir::MemFlagsData::new(),
                table_ptr,
                0,
            );
            lower
                .builder
                .ins()
                .call(register_ref, &[table_ptr, count_val]);
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

    // Define os data symbols para bytes literais.
    for (i, b) in bytes_table.iter().enumerate() {
        let sym = format!("__kata_bytes_{i}");
        let did = module
            .declare_data(&sym, Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift(format!("declare_data {sym}: {e}")))?;
        let mut data_desc = cranelift_module::DataDescription::new();
        data_desc.define(b.clone().into());
        module
            .define_data(did, &data_desc)
            .map_err(|e| CodegenError::Cranelift(format!("define_data {sym}: {e}")))?;
    }

    // ── Emissão da sidecar table para reflexão de funções ──
    // Coletar funções (NÃO actions) do symbol_table. Actions não entram
    // na sidecar table — reflexão de actions é sempre estática via DispatchTable.
    let fn_entries: Vec<(String, Vec<Ty>, Ty, cranelift_module::FuncId)> = typed
        .functions
        .iter()
        .filter_map(|f| {
            let key = (f.name.clone(), f.param_types.clone(), f.ret_ty.clone());
            symbol_table
                .get(&key)
                .map(|&fid| (f.name.clone(), f.param_types.clone(), f.ret_ty.clone(), fid))
        })
        .collect();

    if !fn_entries.is_empty() {
        // Adicionar strings de nome, param_types e return_type à string_table.
        let strings_before = string_table.len();
        for (name, params, ret, _fid) in &fn_entries {
            string_table.push(name.clone());
            for pty in params {
                string_table.push(pty.to_text());
            }
            string_table.push(ret.to_text());
        }
        // Definir as novas strings como data symbols.
        for i in strings_before..string_table.len() {
            let sym = format!("__kata_str_{i}");
            let did = module
                .declare_data(&sym, Linkage::Local, false, false)
                .map_err(|e| CodegenError::Cranelift(format!("declare_data {sym}: {e}")))?;
            let mut data_desc = cranelift_module::DataDescription::new();
            let bytes = format!("{}\0", string_table[i]).into_bytes();
            data_desc.define(bytes.into());
            module
                .define_data(did, &data_desc)
                .map_err(|e| CodegenError::Cranelift(format!("define_data {sym}: {e}")))?;
        }

        // Coletar DataIds das strings para cada função.
        let mut str_idx = strings_before;
        let mut fn_meta_data_ids: Vec<(
            cranelift_module::DataId,
            Vec<cranelift_module::DataId>,
            cranelift_module::DataId,
        )> = Vec::new();
        for (_name, params, _ret, _fid) in &fn_entries {
            let name_did = module
                .declare_data(
                    &format!("__kata_str_{str_idx}"),
                    Linkage::Local,
                    false,
                    false,
                )
                .map_err(|e| CodegenError::Cranelift(format!("declare_data str: {e}")))?;
            str_idx += 1;
            let mut param_dids = Vec::new();
            for _ in 0..params.len() {
                let pdid = module
                    .declare_data(
                        &format!("__kata_str_{str_idx}"),
                        Linkage::Local,
                        false,
                        false,
                    )
                    .map_err(|e| CodegenError::Cranelift(format!("declare_data str: {e}")))?;
                param_dids.push(pdid);
                str_idx += 1;
            }
            let ret_did = module
                .declare_data(
                    &format!("__kata_str_{str_idx}"),
                    Linkage::Local,
                    false,
                    false,
                )
                .map_err(|e| CodegenError::Cranelift(format!("declare_data str: {e}")))?;
            str_idx += 1;
            fn_meta_data_ids.push((name_did, param_dids, ret_did));
        }

        // Construir sub-arrays de param_types (um data symbol por função).
        let mut param_array_ids: Vec<Option<cranelift_module::DataId>> = Vec::new();
        for (i, (_name, params, _ret, _fid)) in fn_entries.iter().enumerate() {
            if params.is_empty() {
                param_array_ids.push(None);
                continue;
            }
            let sym = format!("__kata_fn_meta_params_{i}");
            let did = module
                .declare_data(&sym, Linkage::Local, false, false)
                .map_err(|e| CodegenError::Cranelift(format!("declare_data {sym}: {e}")))?;
            let (_, param_dids, _) = &fn_meta_data_ids[i];
            let mut data_desc = cranelift_module::DataDescription::new();
            data_desc.define_zeroinit(params.len() * 8);
            for (j, pdid) in param_dids.iter().enumerate() {
                let gv = module.declare_data_in_data(*pdid, &mut data_desc);
                data_desc.write_data_addr((j * 8) as u32, gv, 0);
            }
            module
                .define_data(did, &data_desc)
                .map_err(|e| CodegenError::Cranelift(format!("define_data {sym}: {e}")))?;
            param_array_ids.push(Some(did));
        }

        // Construir a sidecar table: header (count: i64) + entries (56 bytes).
        let count = fn_entries.len();
        let table_size = 8 + count * 56;
        let table_did = module
            .declare_data("__kata_fn_meta_table", Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift(format!("declare_data fn_meta_table: {e}")))?;

        // Bytes: header (count) + entries com campos literais e zeros para relocations.
        let mut table_bytes: Vec<u8> = Vec::with_capacity(table_size);
        table_bytes.extend_from_slice(&(count as i64).to_le_bytes());
        for (_name, params, _ret, _fid) in &fn_entries {
            table_bytes.extend_from_slice(&[0u8; 8]); // fn_ptr (relocation)
            table_bytes.extend_from_slice(&[0u8; 8]); // name_ptr (relocation)
            table_bytes.extend_from_slice(&(params.len() as i64).to_le_bytes()); // arity
            table_bytes.extend_from_slice(&[0u8; 8]); // param_types_ptr (relocation)
            table_bytes.extend_from_slice(&(params.len() as i64).to_le_bytes()); // param_types_len
            table_bytes.extend_from_slice(&[0u8; 8]); // return_type_ptr (relocation)
            table_bytes.extend_from_slice(&[0u8; 8]); // is_action (0 = function)
        }
        let mut data_desc = cranelift_module::DataDescription::new();
        data_desc.define(table_bytes.into());

        // Adicionar relocations para cada entry.
        for (i, (_name, params, _ret, fid)) in fn_entries.iter().enumerate() {
            let base = 8 + i * 56;
            // fn_ptr
            let func_ref = module.declare_func_in_data(*fid, &mut data_desc);
            data_desc.write_function_addr(base as u32, func_ref);
            // name_ptr
            let (name_did, _, _) = &fn_meta_data_ids[i];
            let gv = module.declare_data_in_data(*name_did, &mut data_desc);
            data_desc.write_data_addr((base + 8) as u32, gv, 0);
            // param_types_ptr
            if !params.is_empty() {
                if let Some(pa_did) = param_array_ids[i] {
                    let gv = module.declare_data_in_data(pa_did, &mut data_desc);
                    data_desc.write_data_addr((base + 24) as u32, gv, 0);
                }
            }
            // return_type_ptr
            let (_, _, ret_did) = &fn_meta_data_ids[i];
            let gv = module.declare_data_in_data(*ret_did, &mut data_desc);
            data_desc.write_data_addr((base + 40) as u32, gv, 0);
        }

        module
            .define_data(table_did, &data_desc)
            .map_err(|e| CodegenError::Cranelift(format!("define_data fn_meta_table: {e}")))?;
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
