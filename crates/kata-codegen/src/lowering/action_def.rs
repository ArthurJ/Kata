//! Declaração e definição de Actions Kata.
//!
//! Extraído de `module.rs` para separar a responsabilidade de compilação
//! de Actions da orquestração do entry point.

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::TypedAction;
use kata_inference::TypedLogSpec;

use super::expr::lower_expr;
use crate::metadata::MetadataTable;

use super::backend::ModuleBackend;
use super::log::inject_log;
use super::module::{CodegenError, FuncKey, StringTable};

/// Declara uma Action no JITModule (sem definir ainda).
///
/// `cranelift_name` é o nome interno no JITModule — plumbing sem semântica
/// (ex: `__kata_fn_5`). A identidade semântica vive na chave composta do
/// `symbol_table`.
///
/// Assinatura uniforme: `(rt: i64, fiber_arena: i64, caller_arena: i64, args_ptr: i64) -> i64`
/// com `CallConv::Tail`. Todos os params são I64, retorno é sempre I64
/// (Float é bitcast na borda — epílogo da Action e caller).
/// A2: rt é ponteiro para Box<Runtime>, passado pelo scheduler/entry point.
pub(crate) fn declare_kata_action(
    action: &TypedAction,
    cranelift_name: &str,
    module: &mut dyn ModuleBackend,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(CallConv::Tail);
    // A2: ABI uniforme: rt, fiber_arena, caller_arena, args_ptr — todos I64.
    sig.params.push(AbiParam::new(I64)); // rt
    sig.params.push(AbiParam::new(I64)); // fiber_arena
    sig.params.push(AbiParam::new(I64)); // caller_arena
    sig.params.push(AbiParam::new(I64)); // args_ptr
    // Retorno sempre I64 (Float bitcast na borda).
    sig.returns.push(AbiParam::new(I64));
    module
        .declare_function(cranelift_name, Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("declare action {}: {e}", action.name),
        })
}

/// Define (compila o corpo de) uma Action.
///
/// ABI uniforme: `(fiber_arena: i64, caller_arena: i64, args_ptr: i64) -> i64`.
///
/// Prólogo: sem `arena_create` — a arena do fiber é criada pelo scheduler
/// e passada como `params[0]` (fiber_arena). `params[1]` = caller_arena.
/// `params[2]` = args_ptr (ponteiro para tupla de args, ou 0 se Unit).
///
/// Body: extrai elementos da tupla de args_ptr, liga a variáveis, lowera statements.
/// Epílogo: sem `arena_destroy` — o scheduler destrói a arena após o fiber retornar.
/// Se `ret_ty == Float`, faz `bitcast(I64 ← F64)` antes do `return_`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn define_kata_action(
    action: &TypedAction,
    func_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    bytes_table: &mut Vec<Vec<u8>>,
    struct_registry: &kata_core::StructRegistry,
    type_id_map: &HashMap<Ty, i64>,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        // A2: Assinatura uniforme: (rt, fiber_arena, caller_arena, args_ptr) -> i64.
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64)); // rt
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
        let mut kata_refs: HashMap<FuncKey, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (key, &fid) in symbol_table {
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

        // A2: ABI uniforme: params[0] = rt, params[1] = fiber_arena, params[2] = caller_arena, params[3] = args_ptr.
        let rt_value = params[0];
        let fiber_arena = params[1];
        let caller_arena = params[2];
        let args_ptr = params[3];

        let mut lower = super::LowerCtx {
            builder: &mut builder,
            module,
            ffi_refs: &ffi_refs,
            kata_refs: &kata_refs,
            ffi_ids,
            kata_ids: symbol_table,
            metadata: &mut metadata,
            string_table,
            bytes_table,
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
            io_handle_vars: Vec::new(),
            struct_registry,
            type_id_map,
            ipc_broker_fid: None,
            rt: Some(rt_value),
        };
        // Lowera o corpo da Action.
        // O tipo do param é o tipo NATURAL do retorno (F64 para Float, I64 para resto).
        // O bitcast F64→I64 acontece no epilogue, após ler o block param.
        // Se o param fosse I64 mas o body produz F64, o jump falha no verifier.
        let ret_clif_ty = super::resolve_clif_ty(&action.ret_ty, struct_registry);
        let epilogue_block = lower.builder.create_block();
        lower
            .builder
            .append_block_param(epilogue_block, ret_clif_ty);

        // Configura LowerCtx com epilogue_block.
        lower.epilogue_block = Some(epilogue_block);

        // Extrai elementos da tupla de args_ptr e liga a variáveis.
        // O inference define params como __param_0, __param_1, ... no typeck.
        // Se a action usa forma nomeada (`x::Tipo`), o body referencia `x`; o
        // codegen registra `x` como alias da mesma Variable de `__param_N`.
        // args_ptr é um ponteiro para a tupla na arena (ou 0 se Unit).
        let flags = MemFlagsData::new();
        for (i, pt) in action.param_types.iter().enumerate() {
            let clif_ty = super::resolve_clif_ty(pt, struct_registry);
            let var = lower.new_var(&format!("__param_{i}"), clif_ty);
            let offset = (i * 8) as i32;
            let val = lower.builder.ins().load(clif_ty, flags, args_ptr, offset);
            lower.builder.def_var(var, val);
            // Alias: se o param é nomeado (`x::Tipo`), registra `x` apontando
            // para a mesma Variable. O body referencia `x`, não `__param_N`.
            if let Some(Some(name)) = action.param_names.get(i) {
                lower.var_map.insert(name.clone(), var);
            }
        }

        // Body: lowera cada statement em sequência.
        // O último statement é o retorno implícito.
        let n = action.body.len();
        let mut last_result = lower.builder.ins().iconst(I64, 0); // Unit default
        let mut hit_return = false;

        // Injeta @log Enter (prólogo) — pode haver múltiplas diretivas.
        for spec in action
            .log
            .iter()
            .filter(|s| matches!(s, TypedLogSpec::Enter { .. }))
        {
            inject_log(spec, &mut lower)?;
        }

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

        // Define o epilogue_block: decref de ARC vars + return_.
        lower.builder.switch_to_block(epilogue_block);
        lower.builder.seal_block(epilogue_block);
        let result = lower.builder.block_params(epilogue_block)[0];

        // Close de I/O handles abertos que não foram fechados explicitamente.
        // Cada variável segura um handle (ponteiro para FileInner/SocketInner).
        // O close é idempotente — o epílogo despacha por IoHandleKind.
        let file_close_ref = lower
            .ffi_refs
            .get("kata_rt_file_close")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_file_close".into(),
            })?;
        let socket_close_ref = lower
            .ffi_refs
            .get("kata_rt_socket_close")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_socket_close".into(),
            })?;
        for (var, kind) in &lower.io_handle_vars {
            let val = lower.builder.use_var(*var);
            match kind {
                super::IoHandleKind::File => {
                    lower.builder.ins().call(file_close_ref, &[val]);
                }
                super::IoHandleKind::Socket => {
                    lower.builder.ins().call(socket_close_ref, &[val]);
                }
            }
        }

        // Injeta @log Exit (epílogo) — pode haver múltiplas diretivas.
        for spec in action
            .log
            .iter()
            .filter(|s| matches!(s, TypedLogSpec::Exit { .. }))
        {
            inject_log(spec, &mut lower)?;
        }

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

    // Define a função no module — func_id passado diretamente (sem lookup por nome).
    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("define action {}: {e}", action.name),
        })?;
    module.clear_context(&mut ctx);
    Ok(())
}
