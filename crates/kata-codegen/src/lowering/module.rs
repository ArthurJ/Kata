//! Lower do `TypedModule` completo: cria a função `__kata_entry` e
//! retorna o `MetadataTable` sidecar + a string table.
//!
//! Também declara e define funções Kata nomeadas (múltiplas cláusulas).

use std::collections::HashMap;

use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::{TypedFunction, TypedLambdaClause, TypedModule};

use super::clause::{
    all_patterns_are_ident, bind_patterns_to_params, lower_clause_body, lower_clause_chain,
    lower_with_bindings,
};
use super::expr::lower_expr;
use super::LowerCtx;
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
type SymbolTable = HashMap<String, cranelift_module::FuncId>;

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
        };

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
        data_desc.define(s.as_bytes().to_vec().into());
        module
            .define_data(did, &data_desc)
            .map_err(|e| CodegenError::Cranelift(format!("define_data {sym}: {e}")))?;
    }

    Ok((metadata, string_table))
}

/// Declara uma função Kata nomeada no JITModule (sem definir ainda).
fn declare_kata_function(
    func: &TypedFunction,
    module: &mut cranelift_jit::JITModule,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(CallConv::Tail);
    for pt in &func.param_types {
        sig.params.push(AbiParam::new(ty_to_clif(pt)));
    }
    sig.returns.push(AbiParam::new(ty_to_clif(&func.ret_ty)));
    module
        .declare_function(&func.name, Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare kata fn {}: {e}", func.name)))
}

/// Pipeline compartilhado: compila o corpo de uma função Kata (nomeada ou anônima).
///
/// Cria Context + FunctionBuilder, declara FFI/Kata refs, lowera cláusulas
/// (single-Ident fast path ou branch chain), finaliza e define no module.
pub(crate) fn define_function_body(
    name: &str,
    param_types: &[Ty],
    ret_ty: &Ty,
    clauses: &[TypedLambdaClause],
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    kata_ids: &HashMap<String, cranelift_module::FuncId>,
    string_table: &mut StringTable,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        let mut sig = Signature::new(CallConv::Tail);
        for pt in param_types {
            sig.params.push(AbiParam::new(ty_to_clif(pt)));
        }
        sig.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
        func_ir.signature = sig;

        // Declara FFI e funções Kata no Function.
        let mut ffi_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (fname, &fid) in ffi_ids {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            ffi_refs.insert(fname.clone(), func_ref);
        }
        let mut kata_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (fname, &fid) in kata_ids {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            kata_refs.insert(fname.clone(), func_ref);
        }

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(func_ir, &mut func_ctx);

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let params: Vec<cranelift_codegen::ir::Value> =
            builder.block_params(entry_block).to_vec();

        let mut lower = LowerCtx {
            builder: &mut builder,
            module,
            ffi_refs: &ffi_refs,
            kata_refs: &kata_refs,
            ffi_ids,
            kata_ids,
            metadata: &mut metadata,
            string_table,
            var_map: HashMap::new(),
            anon_counter: 0,
            emitted_tail_call: false,
            no_tail_calls: false,
        };

        if clauses.len() == 1 && all_patterns_are_ident(&clauses[0].patterns) {
            let clause = &clauses[0];
            bind_patterns_to_params(&clause.patterns, &params, &mut lower);
            lower_with_bindings(&clause.with_bindings, &mut lower)?;
            lower.emitted_tail_call = false;
            let result = lower_clause_body(clause, &mut lower)?;
            if !lower.emitted_tail_call {
                lower.builder.ins().return_(&[result]);
            }
        } else {
            lower_clause_chain(clauses, &params, &mut lower)?;
        }

        builder.finalize();
    }

    // Define a função no module.
    let func_id = module
        .get_name(name)
        .ok_or_else(|| CodegenError::Cranelift(format!("func {name} not declared")))?;
    let func_id = match func_id {
        cranelift_module::FuncOrDataId::Func(fid) => fid,
        _ => return Err(CodegenError::Cranelift(format!("{name} is not a function"))),
    };
    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift(format!("define fn {name}: {e}")))?;
    module.clear_context(&mut ctx);
    Ok(())
}

/// Define (compila o corpo de) uma função Kata nomeada.
fn define_kata_function(
    func: &TypedFunction,
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &SymbolTable,
    string_table: &mut StringTable,
) -> Result<(), CodegenError> {
    define_function_body(
        &func.name,
        &func.param_types,
        &func.ret_ty,
        &func.clauses,
        module,
        ffi_ids,
        symbol_table,
        string_table,
    )
}