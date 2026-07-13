//! Lower do `TypedModule` completo: cria a função `__kata_entry` e
//! retorna o `MetadataTable` sidecar + a string table.
//!
//! Também orquestra declaração/definição de funções Kata nomeadas e Actions,
//! delegando para `function_def` e `action_def`.

use std::collections::HashMap;

use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_inference::TypedModule;

use super::LowerCtx;
use super::action_def::{declare_kata_action, define_kata_action};
use super::expr::lower_expr;
use super::function_def::{declare_kata_function, define_kata_function};
use crate::ffi_sigs::ty_to_clif;
use crate::metadata::MetadataTable;

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

/// Tabela de símbolos de funções Kata nomeadas — mapeia nome → FuncId.
pub(crate) type SymbolTable = HashMap<String, cranelift_module::FuncId>;

/// Lower do `TypedModule` completo: cria a função `__kata_entry` e
/// retorna o `MetadataTable` sidecar + a string table.
pub(crate) fn lower_module(
    typed: &TypedModule,
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
) -> Result<(MetadataTable, StringTable), CodegenError> {
    let mut metadata = MetadataTable::new();
    let mut string_table = StringTable::new();
    let mut symbol_table: SymbolTable = HashMap::new();

    // ── Fase 9: declara e define funções nomeadas antes do entry point ──
    for func in &typed.functions {
        let func_id = declare_kata_function(func, module)?;
        symbol_table.insert(func.name.clone(), func_id);
    }

    for func in &typed.functions {
        define_kata_function(func, module, ffi_ids, &symbol_table, &mut string_table)?;
    }

    // ── Fio 3: declara e define Actions antes do entry point ──
    for action in &typed.actions {
        let func_id = declare_kata_action(action, module)?;
        symbol_table.insert(action.name.clone(), func_id);
    }

    for action in &typed.actions {
        define_kata_action(action, module, ffi_ids, &symbol_table, &mut string_table)?;
    }

    // Determina o tipo de retorno do entry point.
    let ret_ty = &typed.entry.node.ty;
    let ret_clif = ty_to_clif(ret_ty);

    // Assinatura do __kata_entry: () → ret_clif
    let mut sig = Signature::new(CallConv::SystemV);
    sig.returns.push(AbiParam::new(ret_clif));

    let entry_id = module
        .declare_function("__kata_entry", Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare __kata_entry: {e}")))?;

    // Cria um Context do Cranelift (não FunctionBuilderContext).
    let mut ctx = module.make_context();

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
        let mut kata_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (name, &fid) in &symbol_table {
            let func_ref = module.declare_func_in_func(fid, func);
            kata_refs.insert(name.clone(), func_ref);
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
            closure_captures: HashMap::new(),
        };

        // Prólogo do entry point: inicializa scheduler + cria arena global.
        // A arena global serve como caller_arena para a primeira Action (via spawn).
        let scheduler_init_ref = lower
            .ffi_refs
            .get("kata_rt_scheduler_init")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_scheduler_init".into()))?;
        lower.builder.ins().call(scheduler_init_ref, &[]);

        let arena_create_ref = lower
            .ffi_refs
            .get("kata_rt_arena_create")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_arena_create".into()))?;
        let global_arena = lower.builder.ins().call(arena_create_ref, &[]);
        let global_arena = lower.builder.inst_results(global_arena)[0];
        lower.caller_arena = Some(global_arena);

        // Lowera pre_entry (let bindings e outras expressões top-level anteriores).
        // Estas são loweradas em sequência, compartilhando o var_map —
        // um `let` define uma variável que o entry pode usar.
        for pre in &typed.pre_entry {
            lower_expr(&pre.node, &mut lower)?;
        }

        let result = lower_expr(&typed.entry.node, &mut lower)?;

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

    Ok((metadata, string_table))
}
