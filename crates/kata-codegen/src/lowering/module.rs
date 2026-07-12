//! Lower do `TypedModule` completo: cria a função `__kata_entry` e
//! retorna o `MetadataTable` sidecar + a string table.
//!
//! Também declara e define funções Kata nomeadas (múltiplas cláusulas).

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::{CaptureInfo, TypedAction, TypedFunction, TypedLambdaClause, TypedModule};

use super::LowerCtx;
use super::clause::{
    all_patterns_are_ident, bind_patterns_to_params, lower_clause_body, lower_clause_chain,
    lower_with_bindings,
};
use super::expr::lower_expr;
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn define_function_body(
    name: &str,
    param_types: &[Ty],
    ret_ty: &Ty,
    clauses: &[TypedLambdaClause],
    captures: &[CaptureInfo],
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
        // Se há captures, o primeiro param é box_ptr (I64).
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

        let params: Vec<cranelift_codegen::ir::Value> = builder.block_params(entry_block).to_vec();

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
            epilogue_block: None,
            fiber_arena: None,
            caller_arena: None,
            scheduler_mode: false, // funções puras não chamam Actions
            loop_break_block: None,
            loop_continue_block: None,
            closure_captures: HashMap::new(),
        };

        // Se há captures, carrega cada capture do box_ptr e define variável.
        // Layout do CaptureBox: offset 0 = fn_ptr, offset 8 = refcount,
        // offset 16 + i*8 = captures[i].
        // O box_ptr é o primeiro block param (params[0]).
        let clause_params: Vec<cranelift_codegen::ir::Value> = if !captures.is_empty() {
            let box_ptr = params[0];
            let flags = cranelift_codegen::ir::MemFlagsData::new();
            for (i, cap) in captures.iter().enumerate() {
                let clif_ty = ty_to_clif(&cap.ty);
                let offset = (16 + i * 8) as i32;
                let val = lower.builder.ins().load(clif_ty, flags, box_ptr, offset);
                lower.new_var(&cap.name, clif_ty);
                let var = *lower.var_map.get(&cap.name).unwrap();
                lower.builder.def_var(var, val);
            }
            params[1..].to_vec()
        } else {
            params.clone()
        };

        if clauses.len() == 1 && all_patterns_are_ident(&clauses[0].patterns) {
            let clause = &clauses[0];
            bind_patterns_to_params(&clause.patterns, &clause_params, &mut lower);
            lower_with_bindings(&clause.with_bindings, &mut lower)?;
            lower.emitted_tail_call = false;
            let result = lower_clause_body(clause, &mut lower)?;
            if !lower.emitted_tail_call {
                lower.builder.ins().return_(&[result]);
            }
        } else {
            lower_clause_chain(clauses, &clause_params, &mut lower)?;
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
        &[], // funções nomeadas não têm captures
        module,
        ffi_ids,
        symbol_table,
        string_table,
    )
}
/// Declara uma Action no JITModule (sem definir ainda).
///
/// Assinatura uniforme (Fase 10): `(fiber_arena: i64, caller_arena: i64, args_ptr: i64) -> i64`
/// com `CallConv::Tail`. Todos os params são I64, retorno é sempre I64
/// (Float é bitcast na borda — epílogo da Action e caller).
fn declare_kata_action(
    action: &TypedAction,
    module: &mut cranelift_jit::JITModule,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(CallConv::Tail);
    // ABI uniforme: fiber_arena, caller_arena, args_ptr — todos I64.
    sig.params.push(AbiParam::new(I64)); // fiber_arena
    sig.params.push(AbiParam::new(I64)); // caller_arena
    sig.params.push(AbiParam::new(I64)); // args_ptr
    // Retorno sempre I64 (Float bitcast na borda).
    sig.returns.push(AbiParam::new(I64));
    module
        .declare_function(&action.name, Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare action {}: {e}", action.name)))
}

/// Define (compila o corpo de) uma Action.
///
/// ABI uniforme (Fase 10): `(fiber_arena: i64, caller_arena: i64, args_ptr: i64) -> i64`.
///
/// Prólogo: sem `arena_create` — a arena do fiber é criada pelo scheduler
/// e passada como `params[0]` (fiber_arena). `params[1]` = caller_arena.
/// `params[2]` = args_ptr (ponteiro para tupla de args, ou 0 se Unit).
///
/// Body: extrai elementos da tupla de args_ptr, liga a variáveis, lowera statements.
/// Epílogo: sem `arena_destroy` — o scheduler destrói a arena após o fiber retornar.
/// Se `ret_ty == Float`, faz `bitcast(I64 ← F64)` antes do `return_`.
fn define_kata_action(
    action: &TypedAction,
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &SymbolTable,
    string_table: &mut StringTable,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        // Assinatura uniforme: (fiber_arena, caller_arena, args_ptr) -> i64.
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64)); // fiber_arena
        sig.params.push(AbiParam::new(I64)); // caller_arena
        sig.params.push(AbiParam::new(I64)); // args_ptr
        sig.returns.push(AbiParam::new(I64)); // sempre I64 (Float bitcast na borda)
        func_ir.signature = sig;

        // Declara FFI e funções Kata no Function.
        let mut ffi_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (fname, &fid) in ffi_ids {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            ffi_refs.insert(fname.clone(), func_ref);
        }
        let mut kata_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (fname, &fid) in symbol_table {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            kata_refs.insert(fname.clone(), func_ref);
        }

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(func_ir, &mut func_ctx);

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let params: Vec<cranelift_codegen::ir::Value> = builder.block_params(entry_block).to_vec();

        // ABI uniforme: params[0] = fiber_arena, params[1] = caller_arena, params[2] = args_ptr.
        let fiber_arena = params[0];
        let caller_arena = params[1];
        let args_ptr = params[2];

        let mut lower = LowerCtx {
            builder: &mut builder,
            module,
            ffi_refs: &ffi_refs,
            kata_refs: &kata_refs,
            ffi_ids,
            kata_ids: symbol_table,
            metadata: &mut metadata,
            string_table,
            var_map: HashMap::new(),
            anon_counter: 0,
            emitted_tail_call: false,
            no_tail_calls: false,
            epilogue_block: None,
            fiber_arena: Some(fiber_arena),
            caller_arena: Some(caller_arena),
            scheduler_mode: false, // dentro de Action: ActionCalls são call diretos
            loop_break_block: None,
            loop_continue_block: None,
            closure_captures: HashMap::new(),
        };

        // Cria epilogue_block com 1 block param (result).
        // O tipo do param é o tipo NATURAL do retorno (F64 para Float, I64 para resto).
        // O bitcast F64→I64 acontece no epilogue, após ler o block param.
        // Se o param fosse I64 mas o body produz F64, o jump falha no verifier.
        let ret_clif_ty = ty_to_clif(&action.ret_ty);
        let epilogue_block = lower.builder.create_block();
        lower
            .builder
            .append_block_param(epilogue_block, ret_clif_ty);

        // Configura LowerCtx com epilogue_block.
        lower.epilogue_block = Some(epilogue_block);

        // Extrai elementos da tupla de args_ptr e liga a variáveis nomeadas.
        // O inference define params como __param_0, __param_1, ...
        // args_ptr é um ponteiro para a tupla na arena (ou 0 se Unit).
        let flags = cranelift_codegen::ir::MemFlagsData::new();
        for (i, pt) in action.param_types.iter().enumerate() {
            let clif_ty = ty_to_clif(pt);
            let var = lower.new_var(&format!("__param_{i}"), clif_ty);
            let offset = (i * 8) as i32;
            let val = lower.builder.ins().load(clif_ty, flags, args_ptr, offset);
            lower.builder.def_var(var, val);
        }

        // Body: lowera cada statement em sequência.
        // O último statement é o retorno implícito.
        let n = action.body.len();
        let mut last_result = lower.builder.ins().iconst(I64, 0); // Unit default
        let mut hit_return = false;
        for (i, stmt) in action.body.iter().enumerate() {
            last_result = lower_expr(&stmt.node, &mut lower)?;
            // Se emitiu return (jump para epilogue_block), não continuar.
            if matches!(stmt.node.kind, kata_inference::TypedExprKind::Return(_)) {
                hit_return = true;
                break;
            }
            // Se o último statement emitiu tail call, não continuar.
            if i == n - 1 && lower.emitted_tail_call {
                break;
            }
        }

        // Epílogo: se não terminou via return ou tail call,
        // jump para epilogue_block com o último resultado.
        if !hit_return && !lower.emitted_tail_call {
            lower.builder.ins().jump(
                epilogue_block,
                &[cranelift_codegen::ir::BlockArg::Value(last_result)],
            );
        }

        // Define o epilogue_block: return_ (sem arena_destroy — scheduler destrói).
        lower.builder.switch_to_block(epilogue_block);
        lower.builder.seal_block(epilogue_block);
        let result = lower.builder.block_params(epilogue_block)[0];

        // Float bitcast: se ret_ty == Float, o body produziu F64.
        // A ABI retorna I64 — bitcast F64 → I64 antes do return_.
        let ret_val = if action.ret_ty == Ty::float() {
            lower
                .builder
                .ins()
                .bitcast(I64, MemFlagsData::new(), result)
        } else {
            result
        };
        lower.builder.ins().return_(&[ret_val]);

        builder.finalize();
    }

    // Define a função no module.
    let func_id = module
        .get_name(&action.name)
        .ok_or_else(|| CodegenError::Cranelift(format!("action {} not declared", action.name)))?;
    let func_id = match func_id {
        cranelift_module::FuncOrDataId::Func(fid) => fid,
        _ => {
            return Err(CodegenError::Cranelift(format!(
                "{} is not a function",
                action.name
            )));
        }
    };
    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift(format!("define action {}: {e}", action.name)))?;
    module.clear_context(&mut ctx);
    Ok(())
}
