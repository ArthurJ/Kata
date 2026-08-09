//! Declaração e definição de funções Kata nomeadas.
//!
//! Extraído de `module.rs` para separar a responsabilidade de compilação
//! de funções nomeadas (múltiplas cláusulas) da orquestração do entry point.

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{
    AbiParam, InstBuilder, MemFlagsData, Signature, StackSlotData, StackSlotKind,
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::CacheSpec;
use kata_inference::TimerSpec;
use kata_inference::{CaptureInfo, TypedFunction, TypedLambdaClause, TypedLogSpec};

use super::LowerCtx;
use super::backend::ModuleBackend;
use super::clause::{
    all_patterns_are_ident, bind_patterns_to_params, lower_clause_body, lower_clause_chain,
    lower_with_bindings,
};
use super::log::inject_log;
use super::module::{CodegenError, FuncKey, StringTable};
use super::tail_call::has_tail_pos_call;
use super::timer::{
    inject_timer_start, inject_timer_start_channel, inject_timer_stop, inject_timer_stop_channel,
};
use crate::metadata::MetadataTable;

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
    let expected = super::resolve_clif_ty(ret_ty, lower.struct_registry);
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
    struct_registry: &kata_core::StructRegistry,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(CallConv::Tail);
    // arena_handle é o primeiro param implícito — passado pelo caller
    // (fiber_arena ou caller_arena do contexto de chamada).
    sig.params.push(AbiParam::new(I64));
    // box_ptr: segundo param implícito (ABI uniformizada).
    // Funções nomeadas não têm captures — recebem dummy (iconst 0).
    sig.params.push(AbiParam::new(I64));
    for pt in &func.param_types {
        sig.params
            .push(AbiParam::new(super::resolve_clif_ty(pt, struct_registry)));
    }
    sig.returns.push(AbiParam::new(super::resolve_clif_ty(
        &func.ret_ty,
        struct_registry,
    )));
    module
        .declare_function(cranelift_name, Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift { reason: format!("declare kata fn {}: {e}", func.name) })
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
    cache_spec: &Option<CacheSpec>,
    timer_spec: &Option<TimerSpec>,
    func_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    kata_ids: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    bytes_table: &mut Vec<Vec<u8>>,
    struct_registry: &kata_core::StructRegistry,
    type_id_map: &HashMap<Ty, i64>,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        let mut sig = Signature::new(CallConv::Tail);
        // arena_handle: primeiro param implícito, passado pelo caller.
        sig.params.push(AbiParam::new(I64)); // arena_handle
        // box_ptr: segundo param implícito. Sempre presente na ABI uniformizada
        // — lambdas sem captures recebem box com n_captures=0, funções
        // nomeadas recebem dummy (iconst 0). Isto elimina a necessidade de
        // distinguir box_ptr de fn_ptr no call site.
        sig.params.push(AbiParam::new(I64)); // box_ptr
        for pt in param_types {
            sig.params
                .push(AbiParam::new(super::resolve_clif_ty(pt, struct_registry)));
        }
        sig.returns.push(AbiParam::new(super::resolve_clif_ty(
            ret_ty,
            struct_registry,
        )));
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
            bytes_table,
            var_map: HashMap::new(),
            anon_counter: 0,
            emitted_tail_call: false,
            no_tail_calls: cache_spec.is_some(),
            epilogue_block: None,
            fiber_arena: None,
            caller_arena: None,
            scheduler_mode: false, // funções puras não chamam Actions
            loop_break_block: None,
            loop_continue_block: None,
            io_handle_vars: Vec::new(),
            struct_registry,
            type_id_map,
            ipc_broker_fid: None,
        };

        // params[0] = arena_handle (primeiro param implícito da nova ABI).
        // Setar fiber_arena para que alocações internas (cons, array_alloc,
        // etc.) usem a arena do caller — sem leak, sem arena efêmera.
        let arena_handle = params[0];
        lower.fiber_arena = Some(arena_handle);

        // Sempre há box_ptr (ABI uniformizada). Se há captures, carrega cada
        // capture do box_ptr. Se não, box_ptr é dummy (0) e ignorado.
        // Layout do CaptureBox: offset 0 = fn_ptr, offset 8 = refcount,
        // offset 16 = n_captures, offset 24 + i*8 = captures[i].
        // box_ptr é o segundo block param (params[1]).
        let box_ptr = params[1];
        let clause_params: Vec<cranelift_codegen::ir::Value> = if !captures.is_empty() {
            let flags = MemFlagsData::new();
            for (i, cap) in captures.iter().enumerate() {
                let clif_ty = super::resolve_clif_ty(&cap.ty, struct_registry);
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
            params[2..].to_vec()
        };

        // Bind patterns da primeira cláusula aos clause_params antes de @log
        // e @cache. O @log Enter precisa dos nomes dos params no var_map para
        // resolver o template da mensagem (ex: "entrada n={n}").
        // Para múltiplas cláusulas, lower_clause_chain re-bindará em cada cláusula.
        if !clauses.is_empty() && all_patterns_are_ident(&clauses[0].patterns) {
            bind_patterns_to_params(&clauses[0].patterns, &clause_params, &mut lower);
        }

        // @timer start: injeta antes de tudo (PRD §4.7 ordem).
        // Estratégia: se a função faz tail call (return_call) e não tem
        // @cache, usa canal buffer-1 com policy Drop (first-write-wins)
        // — o start vive na heap e sobrevive à destruição de frames do TCO.
        // Caso contrário, usa stack slot (mais simples, sem overhead de canal).
        let timer_use_channel =
            timer_spec.is_some() && cache_spec.is_none() && has_tail_pos_call(clauses);
        let timer_start_val = if timer_spec.is_some() {
            if timer_use_channel {
                Some(inject_timer_start_channel(&mut lower)?)
            } else {
                Some(inject_timer_start(&mut lower)?)
            }
        } else {
            None
        };

        // Se @log quando Enter, injeta antes do body (prólogo).
        if let Some(TypedLogSpec::Enter { .. }) = log {
            inject_log(
                log.as_ref().expect("log é Some: guardado pelo match Enter"),
                &mut lower,
            )?;
        }

        // Cria epilogue_block se @log Exit ou @timer (para interceptar retornos).
        let mut needs_epilogue =
            matches!(log, Some(TypedLogSpec::Exit { .. })) || timer_spec.is_some();

        // ── @cache: cache lookup no prólogo ──
        // Para funções anotadas com @cache{strategy: "LRU"}, serializa
        // os args, faz cache_lookup. Se hit (≠0), retorna direto.
        // Se miss, executa o body e faz cache_insert no epílogo.
        let cache_handle_val = if cache_spec.is_some() && !clause_params.is_empty() {
            let builder = &mut lower.builder;

            // fn_id: hash canônico de nome + param_types + body.
            // Diferencia bodies diferentes com mesma assinatura (REPL iter)
            // e overloads monomórficos.
            let fn_id = super::cache_key::canonical_fn_id(name, param_types, clauses);
            let fn_id_val = builder.ins().iconst(I64, fn_id);

            // capacity: 256 entradas.
            let cap_val = builder.ins().iconst(I64, 256);

            // cache_get_or_create(arena, fn_id, capacity) → handle
            let get_fn = lower
                .ffi_refs
                .get("kata_rt_cache_get_or_create")
                .expect("kata_rt_cache_get_or_create registrado");
            let handle = builder
                .ins()
                .call(*get_fn, &[arena_handle, fn_id_val, cap_val]);
            let handle_val = builder.inst_results(handle)[0];

            // ── Serializa args via type descriptor ──
            // Para cada param, chama kata_rt_serialize_key que caminha o valor
            // segundo o type descriptor e escreve bytes de conteúdo (não ponteiros).
            // O type descriptor é construído em compile-time e escrito na stack.

            // 1. Construir type descriptors para cada param (compile-time).
            let descriptors: Vec<Vec<u8>> = param_types
                .iter()
                .map(|ty| super::cache_key::build_type_descriptor(ty, struct_registry))
                .collect();

            // 2. Alocar stack space para a key buffer (generoso: 4096 bytes).
            let key_cap = 4096i64;
            let key_sslot = builder.func.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                key_cap as u32,
                8,
            ));
            let key_slot = builder.ins().stack_addr(I64, key_sslot, 0);

            // 3. Serializar cada param na key buffer.
            let serialize_fn = lower
                .ffi_refs
                .get("kata_rt_serialize_key")
                .expect("kata_rt_serialize_key registrado");

            let mut key_offset_val = builder.ins().iconst(I64, 0);

            for (i, param) in clause_params.iter().enumerate() {
                let desc = &descriptors[i];

                // Escrever descriptor na stack (para passar ponteiro ao runtime).
                let desc_sslot = builder.func.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    desc.len() as u32,
                    8,
                ));
                let desc_slot = builder.ins().stack_addr(I64, desc_sslot, 0);
                for (j, &byte) in desc.iter().enumerate() {
                    let byte_val = builder.ins().iconst(I64, byte as i64);
                    builder
                        .ins()
                        .store(MemFlagsData::new(), byte_val, desc_slot, j as i32);
                }

                let desc_len_val = builder.ins().iconst(I64, desc.len() as i64);
                let buf_ptr = builder.ins().iadd(key_slot, key_offset_val);
                let cap_const = builder.ins().iconst(I64, key_cap);
                let remaining = builder.ins().isub(cap_const, key_offset_val);

                // Bitcast do param para I64 se necessário (Float é F64).
                let param_ty = builder.func.dfg.value_type(*param);
                let param_i64 = if param_ty != I64 {
                    builder.ins().bitcast(I64, MemFlagsData::new(), *param)
                } else {
                    *param
                };

                let written = builder.ins().call(
                    *serialize_fn,
                    &[param_i64, desc_slot, desc_len_val, buf_ptr, remaining],
                );
                let written_val = builder.inst_results(written)[0];

                // Avançar offset. Se written < 0 (erro), a key fica incompleta
                // — o lookup não vai encontrar nada, e o insert usa o len real.
                key_offset_val = builder.ins().iadd(key_offset_val, written_val);
            }

            let key_len_val = key_offset_val;

            // cache_lookup(handle, key_ptr, key_len) → 0=miss, ptr=hit
            let lookup_fn = lower
                .ffi_refs
                .get("kata_rt_cache_lookup")
                .expect("kata_rt_cache_lookup registrado");
            let lookup_call = builder
                .ins()
                .call(*lookup_fn, &[handle_val, key_slot, key_len_val]);
            let lookup_result = builder.inst_results(lookup_call)[0];

            // Branch: se hit (≠0), return direto. Se miss (0), continua.
            let hit_block = builder.create_block();
            let miss_block = builder.create_block();
            let zero = builder.ins().iconst(I64, 0);
            let is_hit = builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                lookup_result,
                zero,
            );
            builder.ins().brif(is_hit, hit_block, &[], miss_block, &[]);

            // Hit block: return cached value.
            builder.switch_to_block(hit_block);
            builder.seal_block(hit_block);
            // O valor retornado é o ponteiro do cache — para tipos escalares
            // (Int), é o próprio valor. Para tipos complexos, seria um ponteiro
            // para arena (precisaria de deref, mas para 1.0 só suportamos escalares).
            // Bitcast I64→ret_ty se necessário (Float é F64).
            let ret_clif_ty = super::resolve_clif_ty(ret_ty, struct_registry);
            let cached_val = if ret_clif_ty != I64 {
                builder
                    .ins()
                    .bitcast(ret_clif_ty, MemFlagsData::new(), lookup_result)
            } else {
                lookup_result
            };
            builder.ins().return_(&[cached_val]);

            // Miss block: continua para o body.
            builder.switch_to_block(miss_block);
            builder.seal_block(miss_block);

            // Precisa do epilogue para cache_insert antes do return.
            needs_epilogue = true;

            Some((handle_val, key_slot, key_len_val))
        } else {
            None
        };

        if needs_epilogue {
            let ret_clif_ty = super::resolve_clif_ty(ret_ty, struct_registry);
            let epi = lower.builder.create_block();
            lower.builder.append_block_param(epi, ret_clif_ty);
            lower.epilogue_block = Some(epi);
        }

        if clauses.len() == 1 && all_patterns_are_ident(&clauses[0].patterns) {
            let clause = &clauses[0];
            // Patterns já bindados acima (antes de @log/@cache).
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
                    // Decref de variáveis ARC-managed antes do return.
                    emit_close_io_handles(&mut lower);
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

            // Decref de variáveis ARC-managed antes do return.
            emit_close_io_handles(&mut lower);

            // @cache: insert no epílogo.
            if let Some((handle_val, key_slot, key_len_val)) = &cache_handle_val {
                let insert_fn = lower
                    .ffi_refs
                    .get("kata_rt_cache_insert")
                    .expect("kata_rt_cache_insert registrado");
                // Bitcast result→I64 se necessário (Float é F64).
                let result_ty = lower.builder.func.dfg.value_type(result);
                let result_i64 = if result_ty != I64 {
                    lower
                        .builder
                        .ins()
                        .bitcast(I64, MemFlagsData::new(), result)
                } else {
                    result
                };
                lower.builder.ins().call(
                    *insert_fn,
                    &[*handle_val, *key_slot, *key_len_val, result_i64],
                );
            }

            // @timer: stop + publish no epílogo (PRD §4.7 — após @cache insert).
            if let Some(ts) = timer_spec
                && let Some(start) = timer_start_val
            {
                if timer_use_channel {
                    inject_timer_stop_channel(ts, name, start, &mut lower)?;
                } else {
                    inject_timer_stop(ts, name, start, &mut lower)?;
                }
            }

            let result = coerce_return(result, ret_ty, &mut lower);
            lower.builder.ins().return_(&[result]);
        }

        builder.finalize();
    }

    // Define a função no module — func_id passado diretamente (sem lookup por nome).
    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift { reason: format!("define fn {name}: {e}") })?;
    module.clear_context(&mut ctx);
    Ok(())
}

/// Define (compila o corpo de) uma função Kata nomeada.
#[allow(clippy::too_many_arguments)]
pub(crate) fn define_kata_function(
    func: &TypedFunction,
    func_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    bytes_table: &mut Vec<Vec<u8>>,
    struct_registry: &kata_core::StructRegistry,
    type_id_map: &HashMap<Ty, i64>,
) -> Result<(), CodegenError> {
    define_function_body(
        &func.name,
        &func.param_types,
        &func.ret_ty,
        &func.clauses,
        &[], // funções nomeadas não têm capture
        &func.log,
        &func.cache_spec,
        &func.timer_spec,
        func_id,
        module,
        ffi_ids,
        symbol_table,
        string_table,
        bytes_table,
        struct_registry,
        type_id_map,
    )
}

/// Emite close para cada variável em `io_handle_vars` no epílogo de uma
/// função. I/O handles não fechados explicitamente pelo programador são
/// fechados automaticamente antes do return.
fn emit_close_io_handles(lower: &mut LowerCtx) {
    if lower.io_handle_vars.is_empty() {
        return;
    }
    let file_close_ref = lower
        .ffi_refs
        .get("kata_rt_file_close")
        .copied()
        .unwrap_or_else(|| panic!("kata_rt_file_close não encontrado em ffi_refs"));
    let socket_close_ref = lower
        .ffi_refs
        .get("kata_rt_socket_close")
        .copied()
        .unwrap_or_else(|| panic!("kata_rt_socket_close não encontrado em ffi_refs"));
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
}
