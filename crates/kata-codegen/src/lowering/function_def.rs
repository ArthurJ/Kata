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
use kata_inference::CacheStrategy;
use kata_inference::TimerSpec;
use kata_inference::{CaptureInfo, TypedFunction, TypedLambdaClause};

use super::LowerCtx;
use super::backend::ModuleBackend;
use super::clause::{
    all_patterns_are_ident, bind_patterns_to_params, lower_clause_body, lower_clause_chain,
    lower_with_bindings,
};
use super::module::{CodegenError, FuncKey, StringTable};
use super::tail_call::has_tail_pos_call;
use super::timer::{inject_timer_start, inject_timer_stop};
use crate::metadata::MetadataTable;

/// Verifica se uma função precisa do wrapper/inner split.
///
/// O split ocorre quando a função tem **simultaneamente**:
/// 1. Pelo menos uma intrínseca de epílogo (`@cache`, `@timer`)
/// 2. Pelo menos uma self-call em tail position (`return_call`)
///
/// Funções sem intrínsecas, ou sem tail calls, geram uma função só.
pub(crate) fn needs_split(func: &TypedFunction) -> bool {
    let has_epilogue_intrinsics = func.cache_spec.is_some() || func.timer_spec.is_some();
    has_epilogue_intrinsics && has_tail_pos_call(&func.clauses)
}

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
    linkage: Linkage,
    module: &mut dyn ModuleBackend,
    struct_registry: &kata_core::StructRegistry,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(CallConv::Tail);
    // A2: rt é o primeiro param implícito — ponteiro do Runtime passado
    // pelo caller (Action ou outra função pura). Necessário para que
    // funções puras possam chamar FFIs centrais (arena_alloc, etc.).
    sig.params.push(AbiParam::new(I64)); // rt
    // arena_handle é o segundo param implícito — passado pelo caller
    // (fiber_arena ou caller_arena do contexto de chamada).
    sig.params.push(AbiParam::new(I64));
    // box_ptr: terceiro param implícito (ABI uniformizada).
    // Funções nomeadas não têm capture — recebem dummy (iconst 0).
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
        .declare_function(cranelift_name, linkage, &sig)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("declare kata fn {}: {e}", func.name),
        })
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
    cache_spec: &Option<CacheSpec>,
    timer_spec: &Option<TimerSpec>,
    func_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    kata_ids: &HashMap<FuncKey, cranelift_module::FuncId>,
    inner_kata_ids: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    bytes_table: &mut Vec<Vec<u8>>,
    struct_registry: &kata_core::StructRegistry,
    type_id_map: &HashMap<Ty, i64>,
    dump_ir: bool,
    ir_dump: &mut Vec<(String, String)>,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        let mut sig = Signature::new(CallConv::Tail);
        // A2: rt é o primeiro param implícito — ponteiro do Runtime.
        sig.params.push(AbiParam::new(I64)); // rt
        // arena_handle: segundo param implícito, passado pelo caller.
        sig.params.push(AbiParam::new(I64)); // arena_handle
        // box_ptr: terceiro param implícito. Sempre presente na ABI uniformizada
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
        // Inner refs: declara inner FuncIds no function e popula kata_refs_inner.
        // Para funções sem split, inner_kata_ids é vazio → kata_refs_inner vazio.
        let mut kata_refs_inner: HashMap<FuncKey, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (key, &fid) in inner_kata_ids {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            kata_refs_inner.insert(key.clone(), func_ref);
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
            kata_refs_inner: &kata_refs_inner,
            ffi_ids,
            kata_ids,
            metadata: &mut metadata,
            string_table,
            bytes_table,
            var_map: HashMap::new(),
            anon_counter: 0,
            emitted_tail_call: false,
            emitted_terminator: false,
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
            rt: None,
            dump_ir,
            ir_dump,
        };

        // params[0] = rt (primeiro param implícito da ABI A2).
        let rt_value = params[0];

        // params[1] = arena_handle (segundo param implícito).
        // Setar fiber_arena para que alocações internas (cons, array_alloc,
        // etc.) usem a arena do caller — sem leak, sem arena efêmera.
        let arena_handle = params[1];

        // A2: setar ctx.rt para que escape_arena.rs e FFIs centrais tenham
        // acesso ao ponteiro do Runtime quando chamadas de dentro de funções
        // puras. Isto resolve o bug rt=0 em map/filter/hof.
        lower.rt = Some(rt_value);
        lower.fiber_arena = Some(arena_handle);

        // params[2] = box_ptr (terceiro param implícito, ABI uniformizada).
        // Se há captures, carrega cada capture do box_ptr. Se não, box_ptr é
        // dummy (0) e ignorado.
        // Layout do CaptureBox: offset 0 = fn_ptr, offset 8 = refcount,
        // offset 16 = n_captures, offset 24 + i*8 = captures[i].
        let box_ptr = params[2];
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
            params[3..].to_vec()
        } else {
            params[3..].to_vec()
        };

        // Bind patterns da primeira cláusula aos clause_params antes de @log
        // e @cache. O @log Enter precisa dos nomes dos params no var_map para
        // resolver o template da mensagem (ex: "entrada n={n}").
        // Para múltiplas cláusulas, lower_clause_chain re-bindará em cada cláusula.
        if !clauses.is_empty() && all_patterns_are_ident(&clauses[0].patterns) {
            bind_patterns_to_params(&clauses[0].patterns, &clause_params, &mut lower);
        }

        // Registrar `__param_{i}` no var_map para que diretivas customizadas
        // possam sintetizar `_args := (__param_0, __param_1, ...)` em funções puras.
        // Funções puras não nomeiam params na assinatura por design — `__param_{i}`
        // é o identificador posicional usado pelo desugar de diretivas.
        // O def_var no entry block domina todos os blocks de cláusulas.
        for (i, val) in clause_params.iter().enumerate() {
            let param_name = format!("__param_{i}");
            let clif_ty = super::resolve_clif_ty(&param_types[i], struct_registry);
            lower.new_var(&param_name, clif_ty);
            let var = *lower
                .var_map
                .get(&param_name)
                .expect("__param_{i} var must exist after new_var");
            lower.builder.def_var(var, *val);
        }

        // @timer start: injeta antes de tudo (PRD §4.7 ordem).
        // Sempre usa stack slot. O caso TCO (@timer + tail calls sem @cache)
        // era resolvido com canal buffer-1 Drop, mas agora usa wrapper/inner
        // split (needs_split) — o wrapper tem stack slot e o inner faz TCO.
        // Funções sem split não têm @timer + tail calls (needs_split = true
        // nesse caso), então o stack slot é sempre seguro.
        let timer_start_val = if timer_spec.is_some() {
            Some(inject_timer_start(&mut lower)?)
        } else {
            None
        };

        // Cria epilogue_block se @timer (para interceptar retornos).
        let mut needs_epilogue = timer_spec.is_some();

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

            // capacity: do CacheSpec (default 256).
            let cap_val = builder.ins().iconst(I64, cache_spec.as_ref().map_or(256, |s| s.capacity));

            // strategy_tag: 0=LRU, 1=FIFO, 2=MRU, 3=LFU.
            let strategy_tag = cache_spec.as_ref().map_or(0i64, |s| match s.strategy {
                CacheStrategy::LRU => 0,
                CacheStrategy::FIFO => 1,
                CacheStrategy::MRU => 2,
                CacheStrategy::LFU => 3,
            });
            let strategy_tag_val = builder.ins().iconst(I64, strategy_tag);

            // cache_get_or_create(arena, fn_id, capacity, strategy_tag) → handle
            let get_fn = lower
                .ffi_refs
                .get("kata_rt_cache_get_or_create")
                .expect("kata_rt_cache_get_or_create registrado");
            let handle = builder
                .ins()
                .call(*get_fn, &[arena_handle, fn_id_val, cap_val, strategy_tag_val]);
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
            lower.emitted_terminator = false;
            let result = lower_clause_body(clause, &mut lower)?;
            if !lower.emitted_terminator && !lower.emitted_tail_call {
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
                inject_timer_stop(ts, name, start, &mut lower)?;
            }

            let result = coerce_return(result, ret_ty, &mut lower);
            lower.builder.ins().return_(&[result]);
        }

        builder.finalize();
    }

    // Define a função no module — func_id passado diretamente (sem lookup por nome).
    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("define fn {name}: {e}"),
        })?;
    if dump_ir {
        ir_dump.push((name.to_string(), format!("{}", ctx.func.display())));
    }
    module.clear_context(&mut ctx);
    Ok(())
}

/// Define (compila o corpo de) uma função Kata nomeada.
///
/// Se `needs_split(func)` e há inner FuncId em `inner_table`, define duas
/// funções: o inner (body puro com TCO) e o wrapper (prólogo/epílogo com
/// intrínsecas, `call inner`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn define_kata_function(
    func: &TypedFunction,
    func_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    inner_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    bytes_table: &mut Vec<Vec<u8>>,
    struct_registry: &kata_core::StructRegistry,
    type_id_map: &HashMap<Ty, i64>,
    dump_ir: bool,
    ir_dump: &mut Vec<(String, String)>,
) -> Result<(), CodegenError> {
    let key = (
        func.name.clone(),
        func.param_types.clone(),
        func.ret_ty.clone(),
    );

    if let Some(&inner_id) = inner_table.get(&key) {
        // ── Wrapper/inner split ──
        // 1. Definir inner: body puro com TCO, sem intrínsecas.
        //    kata_ids = symbol_table (wrapper) → non-tail self-calls vão ao wrapper (cache).
        //    inner_kata_ids = {key → inner_id} → tail self-calls vão ao inner (TCO).
        let mut inner_ids_map = HashMap::new();
        inner_ids_map.insert(key.clone(), inner_id);
        define_function_body(
            &func.name,
            &func.param_types,
            &func.ret_ty,
            &func.clauses,
            &[],
            &None, // inner não tem cache
            &None, // inner não tem timer
            inner_id,
            module,
            ffi_ids,
            symbol_table,
            &inner_ids_map,
            string_table,
            bytes_table,
            struct_registry,
            type_id_map,
            dump_ir,
            ir_dump,
        )?;

        // 2. Definir wrapper: prólogo (intrínsecas) → call inner → epílogo.
        define_wrapper(
            func,
            func_id,
            inner_id,
            module,
            ffi_ids,
            symbol_table,
            string_table,
            bytes_table,
            struct_registry,
            type_id_map,
            dump_ir,
            ir_dump,
        )?;
    } else {
        // ── Sem split: approach atual ──
        let empty_inner: HashMap<FuncKey, cranelift_module::FuncId> = HashMap::new();
        define_function_body(
            &func.name,
            &func.param_types,
            &func.ret_ty,
            &func.clauses,
            &[],
            &func.cache_spec,
            &func.timer_spec,
            func_id,
            module,
            ffi_ids,
            symbol_table,
            &empty_inner,
            string_table,
            bytes_table,
            struct_registry,
            type_id_map,
            dump_ir,
            ir_dump,
        )?;
    }

    Ok(())
}

/// Define o wrapper no wrapper/inner split.
///
/// O wrapper é uma função Cranelift com a mesma assinatura da original,
/// mas o body é substituído por:
/// 1. Bind params (todos Ident — padrão único)
/// 2. Prólogo: @timer start → @cache lookup (hit? return cached)
/// 3. `call inner(rt, arena, box_ptr, args...)` — call comum (não return_call)
/// 4. Epílogo: @cache insert → @timer stop + publish
/// 5. return result
///
/// O wrapper tem `no_tail_calls = true` mas só faz `call inner` (sem return_call).
/// O frame do wrapper sobrevive — o epílogo executa normalmente.
#[allow(clippy::too_many_arguments)]
fn define_wrapper(
    func: &TypedFunction,
    func_id: cranelift_module::FuncId,
    inner_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    bytes_table: &mut Vec<Vec<u8>>,
    struct_registry: &kata_core::StructRegistry,
    type_id_map: &HashMap<Ty, i64>,
    dump_ir: bool,
    ir_dump: &mut Vec<(String, String)>,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64)); // rt
        sig.params.push(AbiParam::new(I64)); // arena_handle
        sig.params.push(AbiParam::new(I64)); // box_ptr
        for pt in &func.param_types {
            sig.params
                .push(AbiParam::new(super::resolve_clif_ty(pt, struct_registry)));
        }
        sig.returns.push(AbiParam::new(super::resolve_clif_ty(
            &func.ret_ty,
            struct_registry,
        )));
        func_ir.signature = sig;

        // Declara FFI no function.
        let mut ffi_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (fname, &fid) in ffi_ids {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            ffi_refs.insert(fname.clone(), func_ref);
        }

        // Declara funções Kata (symbol_table) no function — para cache lookup etc.
        let mut kata_refs: HashMap<FuncKey, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (key, &fid) in symbol_table {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            kata_refs.insert(key.clone(), func_ref);
        }

        // Declara o inner no function — para o `call inner`.
        let inner_ref = module.declare_func_in_func(inner_id, func_ir);

        let mut kata_refs_inner: HashMap<FuncKey, cranelift_codegen::ir::FuncRef> = HashMap::new();
        let key = (
            func.name.clone(),
            func.param_types.clone(),
            func.ret_ty.clone(),
        );
        kata_refs_inner.insert(key, inner_ref);

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
            kata_refs_inner: &kata_refs_inner,
            ffi_ids,
            kata_ids: symbol_table,
            metadata: &mut metadata,
            string_table,
            bytes_table,
            var_map: HashMap::new(),
            anon_counter: 0,
            emitted_tail_call: false,
            emitted_terminator: false,
            no_tail_calls: true, // wrapper nunca faz return_call
            epilogue_block: None,
            fiber_arena: None,
            caller_arena: None,
            scheduler_mode: false,
            loop_break_block: None,
            loop_continue_block: None,
            io_handle_vars: Vec::new(),
            struct_registry,
            type_id_map,
            ipc_broker_fid: None,
            rt: None,
            dump_ir,
            ir_dump,
        };

        let rt_value = params[0];
        let arena_handle = params[1];
        let box_ptr = params[2];
        let clause_params = params[3..].to_vec();

        lower.rt = Some(rt_value);
        lower.fiber_arena = Some(arena_handle);

        // Bind patterns (todos Ident — padrão único na wrapper, que só repassa args).
        if !func.clauses.is_empty()
            && super::clause::all_patterns_are_ident(&func.clauses[0].patterns)
        {
            super::clause::bind_patterns_to_params(
                &func.clauses[0].patterns,
                &clause_params,
                &mut lower,
            );
        }

        // Registrar __param_{i} no var_map (mesmo que define_function_body).
        for (i, val) in clause_params.iter().enumerate() {
            let param_name = format!("__param_{i}");
            let clif_ty = super::resolve_clif_ty(&func.param_types[i], struct_registry);
            lower.new_var(&param_name, clif_ty);
            let var = *lower
                .var_map
                .get(&param_name)
                .expect("__param_{i} var must exist after new_var");
            lower.builder.def_var(var, *val);
        }

        // ── Prólogo: @timer start ──
        // Wrapper sempre usa stack slot (nunca canal — o frame sobrevive).
        let timer_start_val = if func.timer_spec.is_some() {
            Some(inject_timer_start(&mut lower)?)
        } else {
            None
        };

        let mut needs_epilogue = func.timer_spec.is_some();

        // ── Prólogo: @cache lookup ──
        let cache_handle_val = if func.cache_spec.is_some() && !clause_params.is_empty() {
            let builder = &mut lower.builder;

            let fn_id = super::cache_key::canonical_fn_id(
                &func.name,
                &func.param_types,
                &func.clauses,
            );
            let fn_id_val = builder.ins().iconst(I64, fn_id);

            let cap_val = builder
                .ins()
                .iconst(I64, func.cache_spec.as_ref().map_or(256, |s| s.capacity));

            let strategy_tag = func.cache_spec.as_ref().map_or(0i64, |s| match s.strategy {
                CacheStrategy::LRU => 0,
                CacheStrategy::FIFO => 1,
                CacheStrategy::MRU => 2,
                CacheStrategy::LFU => 3,
            });
            let strategy_tag_val = builder.ins().iconst(I64, strategy_tag);

            let get_fn = lower
                .ffi_refs
                .get("kata_rt_cache_get_or_create")
                .expect("kata_rt_cache_get_or_create registrado");
            let handle = builder
                .ins()
                .call(*get_fn, &[arena_handle, fn_id_val, cap_val, strategy_tag_val]);
            let handle_val = builder.inst_results(handle)[0];

            // Serializa args (mesmo que define_function_body).
            let descriptors: Vec<Vec<u8>> = func
                .param_types
                .iter()
                .map(|ty| super::cache_key::build_type_descriptor(ty, struct_registry))
                .collect();

            let key_cap = 4096i64;
            let key_sslot = builder.func.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                key_cap as u32,
                8,
            ));
            let key_slot = builder.ins().stack_addr(I64, key_sslot, 0);

            let serialize_fn = lower
                .ffi_refs
                .get("kata_rt_serialize_key")
                .expect("kata_rt_serialize_key registrado");

            let mut key_offset_val = builder.ins().iconst(I64, 0);

            for (i, param) in clause_params.iter().enumerate() {
                let desc = &descriptors[i];

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

                key_offset_val = builder.ins().iadd(key_offset_val, written_val);
            }

            let key_len_val = key_offset_val;

            let lookup_fn = lower
                .ffi_refs
                .get("kata_rt_cache_lookup")
                .expect("kata_rt_cache_lookup registrado");
            let lookup_call = builder
                .ins()
                .call(*lookup_fn, &[handle_val, key_slot, key_len_val]);
            let lookup_result = builder.inst_results(lookup_call)[0];

            // Branch: hit → return cached. miss → call inner.
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
            let ret_clif_ty = super::resolve_clif_ty(&func.ret_ty, struct_registry);
            let cached_val = if ret_clif_ty != I64 {
                builder
                    .ins()
                    .bitcast(ret_clif_ty, MemFlagsData::new(), lookup_result)
            } else {
                lookup_result
            };
            builder.ins().return_(&[cached_val]);

            // Miss block: continua para call inner.
            builder.switch_to_block(miss_block);
            builder.seal_block(miss_block);

            needs_epilogue = true;
            Some((handle_val, key_slot, key_len_val))
        } else {
            None
        };

        if needs_epilogue {
            let ret_clif_ty = super::resolve_clif_ty(&func.ret_ty, struct_registry);
            let epi = lower.builder.create_block();
            lower.builder.append_block_param(epi, ret_clif_ty);
            lower.epilogue_block = Some(epi);
        }

        // ── call inner(rt, arena, box_ptr, args...) ──
        let rt_val = lower.rt.unwrap_or_else(|| lower.builder.ins().iconst(I64, 0));
        let arena = lower
            .fiber_arena
            .or(lower.caller_arena)
            .unwrap_or_else(|| lower.builder.ins().iconst(I64, 0));
        let mut inner_call_args = vec![rt_val, arena, box_ptr];
        inner_call_args.extend(clause_params.iter().copied());
        let inner_call = lower.builder.ins().call(inner_ref, &inner_call_args);
        let inner_result = lower.builder.inst_results(inner_call)[0];

        // Jump para epilogue com o resultado.
        if needs_epilogue {
            let result = coerce_return(inner_result, &func.ret_ty, &mut lower);
            lower.builder.ins().jump(
                lower
                    .epilogue_block
                    .expect("epilogue_block definido quando needs_epilogue"),
                &[cranelift_codegen::ir::BlockArg::Value(result)],
            );
        } else {
            let result = coerce_return(inner_result, &func.ret_ty, &mut lower);
            emit_close_io_handles(&mut lower);
            lower.builder.ins().return_(&[result]);
        }

        // ── Epilogue block: cache_insert + timer_stop + return ──
        if needs_epilogue {
            let epi = lower
                .epilogue_block
                .expect("epilogue_block definido quando needs_epilogue");
            lower.builder.switch_to_block(epi);
            lower.builder.seal_block(epi);
            let result = lower.builder.block_params(epi)[0];

            emit_close_io_handles(&mut lower);

            // @cache insert.
            if let Some((handle_val, key_slot, key_len_val)) = &cache_handle_val {
                let insert_fn = lower
                    .ffi_refs
                    .get("kata_rt_cache_insert")
                    .expect("kata_rt_cache_insert registrado");
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

            // @timer stop + publish.
            if let Some(ts) = &func.timer_spec
                && let Some(start) = timer_start_val
            {
                inject_timer_stop(ts, &func.name, start, &mut lower)?;
            }

            let result = coerce_return(result, &func.ret_ty, &mut lower);
            lower.builder.ins().return_(&[result]);
        }

        builder.finalize();
    }

    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("define wrapper {}: {e}", func.name),
        })?;
    if dump_ir {
        ir_dump.push((format!("{}__wrapper", func.name), format!("{}", ctx.func.display())));
    }
    module.clear_context(&mut ctx);
    Ok(())
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
