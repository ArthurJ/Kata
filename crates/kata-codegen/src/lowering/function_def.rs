//! Declaração e definição de funções Kata nomeadas.
//!
//! Extraído de `module.rs` para separar a responsabilidade de compilação
//! de funções nomeadas (múltiplas cláusulas) da orquestração do entry point.

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::{CaptureInfo, TypedFunction, TypedLambdaClause, TypedLogSpec};

use super::clause::{
    all_patterns_are_ident, bind_patterns_to_params, lower_clause_body, lower_clause_chain,
    lower_with_bindings,
};
use crate::ffi_sigs::ty_to_clif;
use crate::metadata::MetadataTable;

use super::LowerCtx;
use super::backend::ModuleBackend;
use super::log::inject_log;
use super::module::{CodegenError, FuncKey, StringTable};

/// Bitcast na borda de retorno.
///
/// Se o `ret_ty` mapeia para I64 mas o `result` é F64 (alias de Float),
/// faz bitcast F64→I64. Necessário para construtores identity de alias
/// de primitivos Float, onde o body retorna F64 mas a assinatura
/// retorna I64 (`Ty::Struct` → I64).
fn coerce_return(
    result: cranelift_codegen::ir::Value,
    ret_ty: &Ty,
    lower: &mut LowerCtx,
) -> cranelift_codegen::ir::Value {
    let expected = ty_to_clif(ret_ty);
    let actual = lower.builder.func.dfg.value_type(result);
    if expected != actual {
        lower
            .builder
            .ins()
            .bitcast(expected, MemFlagsData::new(), result)
    } else {
        result
    }
}

/// Declara uma função Kata nomeada no JITModule (sem definir ainda).
///
/// `cranelift_name` é o nome interno no JITModule — plumbing sem semântica
/// (ex: `__kata_fn_0`). A identidade semântica vive na chave composta do
/// `symbol_table`.
pub(crate) fn declare_kata_function(
    func: &TypedFunction,
    cranelift_name: &str,
    module: &mut dyn ModuleBackend,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(CallConv::Tail);
    // arena_handle é o primeiro param implícito — passado pelo caller
    // (fiber_arena ou caller_arena do contexto de chamada).
    sig.params.push(AbiParam::new(I64));
    for pt in &func.param_types {
        sig.params.push(AbiParam::new(ty_to_clif(pt)));
    }
    sig.returns.push(AbiParam::new(ty_to_clif(&func.ret_ty)));
    module
        .declare_function(cranelift_name, Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare kata fn {}: {e}", func.name)))
}

/// Pipeline compartilhado: compila o corpo de uma função Kata (nomeada ou anônima).
///
/// Cria Context + FunctionBuilder, declara FFI/Kata refs, lowera cláusulas
/// (single-Ident fast path ou branch chain), finaliza e define no module.
#[allow(clippy::too_many_arguments)]
pub(crate) fn define_function_body(
    name: &str,
    param_types: &[Ty],
    ret_ty: &Ty,
    clauses: &[TypedLambdaClause],
    captures: &[CaptureInfo],
    log: &Option<TypedLogSpec>,
    func_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    kata_ids: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        let mut sig = Signature::new(CallConv::Tail);
        // arena_handle: primeiro param implícito, passado pelo caller.
        sig.params.push(AbiParam::new(I64)); // arena_handle
        // Se há captures, o segundo param é box_ptr (I64).
        if !captures.is_empty() {
            sig.params.push(AbiParam::new(I64)); // box_ptr
        }
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
        let mut kata_refs: HashMap<FuncKey, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (key, &fid) in kata_ids {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            kata_refs.insert(key.clone(), func_ref);
        }

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(func_ir, &mut func_ctx);

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let params: Vec<cranelift_codegen::ir::Value> = builder.block_params(entry_block).to_vec();

        let mut lower = super::LowerCtx {
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
            epilogue_block: None,
            fiber_arena: None,
            caller_arena: None,
            scheduler_mode: false, // funções puras não chamam Actions
            loop_break_block: None,
            loop_continue_block: None,
            closure_captures: HashMap::new(),
            arc_vars: Vec::new(),
        };

        // params[0] = arena_handle (primeiro param implícito da nova ABI).
        // Setar fiber_arena para que alocações internas (cons, array_alloc,
        // etc.) usem a arena do caller — sem leak, sem arena efêmera.
        let arena_handle = params[0];
        lower.fiber_arena = Some(arena_handle);

        // Se há captures, carrega cada capture do box_ptr e define variável.
        // Layout do CaptureBox (Fio 16): offset 0 = fn_ptr, offset 8 = refcount,
        // offset 16 = n_captures, offset 24 + i*8 = captures[i].
        // box_ptr é o segundo block param (params[1]).
        let clause_params: Vec<cranelift_codegen::ir::Value> = if !captures.is_empty() {
            let box_ptr = params[1];
            let flags = MemFlagsData::new();
            for (i, cap) in captures.iter().enumerate() {
                let clif_ty = ty_to_clif(&cap.ty);
                let offset = (24 + i * 8) as i32;
                let val = lower.builder.ins().load(clif_ty, flags, box_ptr, offset);
                lower.new_var(&cap.name, clif_ty);
                let var = *lower
                    .var_map
                    .get(&cap.name)
                    .expect("capture var must exist in var_map after new_var");
                lower.builder.def_var(var, val);
            }
            params[2..].to_vec()
        } else {
            params[1..].to_vec()
        };

        // Se @log quando Enter, injeta antes do body (prólogo).
        if let Some(TypedLogSpec::Enter { .. }) = log {
            inject_log(
                log.as_ref().expect("log é Some: guardado pelo match Enter"),
                &mut lower,
            )?;
        }

        // Cria epilogue_block se @log Exit (para interceptar retornos).
        let needs_epilogue = matches!(log, Some(TypedLogSpec::Exit { .. }));
        if needs_epilogue {
            let ret_clif_ty = ty_to_clif(ret_ty);
            let epi = lower.builder.create_block();
            lower.builder.append_block_param(epi, ret_clif_ty);
            lower.epilogue_block = Some(epi);
        }

        if clauses.len() == 1 && all_patterns_are_ident(&clauses[0].patterns) {
            let clause = &clauses[0];
            bind_patterns_to_params(&clause.patterns, &clause_params, &mut lower);
            lower_with_bindings(&clause.with_bindings, &mut lower)?;
            lower.emitted_tail_call = false;
            let result = lower_clause_body(clause, &mut lower)?;
            if !lower.emitted_tail_call {
                let result = coerce_return(result, ret_ty, &mut lower);
                if needs_epilogue {
                    // Jump para epilogue_block em vez de return_ direto.
                    lower.builder.ins().jump(
                        lower
                            .epilogue_block
                            .expect("epilogue_block definido quando needs_epilogue"),
                        &[cranelift_codegen::ir::BlockArg::Value(result)],
                    );
                } else {
                    lower.builder.ins().return_(&[result]);
                }
            }
        } else {
            lower_clause_chain(clauses, &clause_params, &mut lower)?;
        }

        // Define o epilogue_block se criado: injeta log + return_.
        if needs_epilogue {
            let epi = lower
                .epilogue_block
                .expect("epilogue_block definido quando needs_epilogue");
            lower.builder.switch_to_block(epi);
            lower.builder.seal_block(epi);
            let result = lower.builder.block_params(epi)[0];

            // Injeta log no epílogo.
            if let Some(TypedLogSpec::Exit { .. }) = log {
                inject_log(
                    log.as_ref().expect("log é Some: guardado pelo match Exit"),
                    &mut lower,
                )?;
            }

            let result = coerce_return(result, ret_ty, &mut lower);
            lower.builder.ins().return_(&[result]);
        }

        builder.finalize();
    }

    // Define a função no module — func_id passado diretamente (sem lookup por nome).
    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift(format!("define fn {name}: {e}")))?;
    module.clear_context(&mut ctx);
    Ok(())
}

/// Define (compila o corpo de) uma função Kata nomeada.
pub(crate) fn define_kata_function(
    func: &TypedFunction,
    func_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
) -> Result<(), CodegenError> {
    define_function_body(
        &func.name,
        &func.param_types,
        &func.ret_ty,
        &func.clauses,
        &[], // funções nomeadas não têm capture
        &func.log,
        func_id,
        module,
        ffi_ids,
        symbol_table,
        string_table,
    )
}
