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
use super::expr::lower_expr;
use super::module::{CodegenError, FuncKey, StringTable};
use super::tail_call::has_tail_pos_call;
use super::timer::{inject_timer_start, inject_timer_stop};
use crate::metadata::MetadataTable;

/// Verifica se uma função precisa do wrapper/inner split.
///
/// O split ocorre quando a função tem **simultaneamente**:
/// 1. Pelo menos uma self-call em tail position (`return_call`)
/// 2. Conteúdo para o wrapper além de `call inner; return`:
///    - Intrínsecas chumbadas (`@cache`, `@timer`)
///    - `synthetic_pre`/`synthetic_post` não-vazios (diretivas customizadas)
///
/// Funções sem tail calls, ou sem conteúdo para o wrapper, geram uma função só.
pub(crate) fn needs_split(func: &TypedFunction) -> bool {
    if !has_tail_pos_call(&func.clauses) {
        return false;
    }
    let has_chumbed = func.cache_spec.is_some() || func.timer_spec.is_some();
    let has_custom = func
        .clauses
        .iter()
        .any(|c| !c.synthetic_pre.is_empty() || !c.synthetic_post.is_empty());
    has_chumbed || has_custom
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

/// O que o "body" de uma função compila: cláusulas do usuário ou `call inner`.
pub(crate) enum BodyKind {
    /// Lowera as cláusulas (single-Ident fast path ou clause chain).
    Clauses,
    /// Faz `call inner(rt, arena, box_ptr, args...)` — wrapper do split.
    /// O `inner_id` é declarado no function dentro de `define_function_body`.
    CallInner { inner_id: cranelift_module::FuncId },
}

/// Resultado do prólogo: valores que o epílogo precisa.
struct PrologueResult {
    /// (handle, key_slot, key_len) se @cache está ativo.
    cache_handle: Option<(
        cranelift_codegen::ir::Value,
        cranelift_codegen::ir::Value,
        cranelift_codegen::ir::Value,
    )>,
    /// Valor de start do timer se @timer está ativo.
    timer_start: Option<cranelift_codegen::ir::Value>,
    /// Se o epilogue_block foi criado (precisa de cache_insert/timer_stop/synthetic_post).
    needs_epilogue: bool,
}

/// Lowera o prólogo: synthetic_pre → timer start → cache lookup.
///
/// Retorna os valores que o epílogo precisa (cache handle, timer start).
/// `clauses_for_cache` é usado para `canonical_fn_id` — no wrapper, passa
/// `func.clauses`; no inner, passa `inner_clauses` (mas inner não tem cache).
#[allow(clippy::too_many_arguments)]
fn lower_prologue(
    lower: &mut LowerCtx,
    name: &str,
    param_types: &[Ty],
    ret_ty: &Ty,
    clauses: &[TypedLambdaClause],
    clause_params: &[cranelift_codegen::ir::Value],
    arena_handle: cranelift_codegen::ir::Value,
    cache_spec: &Option<CacheSpec>,
    timer_spec: &Option<TimerSpec>,
    struct_registry: &kata_core::StructRegistry,
) -> Result<PrologueResult, CodegenError> {
    // synthetic_pre (diretivas Enter customizadas): lowera antes de timer/cache.
    let has_synthetic_pre = clauses.iter().any(|c| !c.synthetic_pre.is_empty());
    let has_synthetic_post = clauses.iter().any(|c| !c.synthetic_post.is_empty());
    if has_synthetic_pre {
        for pre_expr in &clauses[0].synthetic_pre {
            lower_expr(&pre_expr.node, lower)?;
        }
    }

    // @timer start.
    let timer_start = if timer_spec.is_some() {
        Some(inject_timer_start(lower)?)
    } else {
        None
    };

    let mut needs_epilogue = timer_spec.is_some() || has_synthetic_post;

    // @cache lookup.
    let cache_handle = if cache_spec.is_some() && !clause_params.is_empty() {
        let builder = &mut lower.builder;

        let fn_id = super::cache_key::canonical_fn_id(name, param_types, clauses);
        let fn_id_val = builder.ins().iconst(I64, fn_id);
        let cap_val = builder
            .ins()
            .iconst(I64, cache_spec.as_ref().map_or(256, |s| s.capacity));
        let strategy_tag = cache_spec.as_ref().map_or(0i64, |s| match s.strategy {
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
        let handle = builder.ins().call(
            *get_fn,
            &[arena_handle, fn_id_val, cap_val, strategy_tag_val],
        );
        let handle_val = builder.inst_results(handle)[0];

        // Serializa args via type descriptor.
        let descriptors: Vec<Vec<u8>> = param_types
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

        // Branch: hit → return cached. miss → continua.
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

        needs_epilogue = true;
        Some((handle_val, key_slot, key_len_val))
    } else {
        None
    };

    Ok(PrologueResult {
        cache_handle,
        timer_start,
        needs_epilogue,
    })
}

/// Lowera o epílogo: cache_insert → timer_stop → synthetic_post → return.
fn lower_epilogue(
    lower: &mut LowerCtx,
    name: &str,
    ret_ty: &Ty,
    clauses: &[TypedLambdaClause],
    timer_spec: &Option<TimerSpec>,
    prologue: &PrologueResult,
) -> Result<(), CodegenError> {
    let epi = lower
        .epilogue_block
        .expect("epilogue_block definido quando needs_epilogue");
    lower.builder.switch_to_block(epi);
    lower.builder.seal_block(epi);
    let result = lower.builder.block_params(epi)[0];

    emit_close_io_handles(lower);

    // @cache insert.
    if let Some((handle_val, key_slot, key_len_val)) = &prologue.cache_handle {
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
    if let Some(ts) = timer_spec
        && let Some(start) = prologue.timer_start
    {
        inject_timer_stop(ts, name, start, lower)?;
    }

    // synthetic_post (diretivas Exit customizadas).
    let has_synthetic_post = clauses.iter().any(|c| !c.synthetic_post.is_empty());
    if has_synthetic_post {
        let ret_clif_ty = super::resolve_clif_ty(ret_ty, lower.struct_registry);
        lower.new_var("_return", ret_clif_ty);
        let return_var = *lower
            .var_map
            .get("_return")
            .expect("_return var must exist after new_var");
        lower.builder.def_var(return_var, result);

        for post_expr in &clauses[0].synthetic_post {
            lower_expr(&post_expr.node, lower)?;
        }
    }

    let result = coerce_return(result, ret_ty, lower);
    lower.builder.ins().return_(&[result]);
    Ok(())
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
    module
        .declare_function(cranelift_name, linkage, &sig)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("declare kata fn {}: {e}", func.name),
        })
}

/// Compila o corpo de uma função Kata (nomeada ou anônima).
///
/// Pipeline unificado: prólogo (synthetic_pre + timer + cache) → body
/// (cláusulas ou `call inner`) → epílogo (cache_insert + timer_stop +
/// synthetic_post + return).
///
/// `body_kind` distingue:
/// - `Clauses`: lowera as cláusulas do usuário (função sem split, ou inner).
/// - `CallInner`: faz `call inner` (wrapper do split).
///
/// `no_tail_calls`: wrapper do split usa `true` (nunca return_call); inner
/// e funções sem split usam `false` (TCO ativo).
#[allow(clippy::too_many_arguments)]
pub(crate) fn define_function_body(
    name: &str,
    param_types: &[Ty],
    ret_ty: &Ty,
    clauses: &[TypedLambdaClause],
    captures: &[CaptureInfo],
    cache_spec: &Option<CacheSpec>,
    timer_spec: &Option<TimerSpec>,
    body_kind: BodyKind,
    no_tail_calls: bool,
    func_id: cranelift_module::FuncId,
    ir_name: &str,
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
        sig.params.push(AbiParam::new(I64)); // rt
        sig.params.push(AbiParam::new(I64)); // arena_handle
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
        let mut kata_refs_inner: HashMap<FuncKey, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (key, &fid) in inner_kata_ids {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            kata_refs_inner.insert(key.clone(), func_ref);
        }
        // Para BodyKind::CallInner, declarar o inner_ref aqui (antes do builder).
        let call_inner_ref: Option<cranelift_codegen::ir::FuncRef> = match &body_kind {
            BodyKind::CallInner { inner_id } => {
                Some(module.declare_func_in_func(*inner_id, func_ir))
            }
            BodyKind::Clauses => None,
        };

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
            no_tail_calls,
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
        lower.rt = Some(rt_value);
        lower.fiber_arena = Some(arena_handle);

        // Captures: carrega do box_ptr (apenas Clauses — wrapper não tem captures).
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

        // Bind patterns da primeira cláusula antes de @log/@cache.
        if !clauses.is_empty() && all_patterns_are_ident(&clauses[0].patterns) {
            bind_patterns_to_params(&clauses[0].patterns, &clause_params, &mut lower);
        }

        // Registrar __param_{i} no var_map para diretivas customizadas.
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

        // ── Prólogo ──
        let prologue = lower_prologue(
            &mut lower,
            name,
            param_types,
            ret_ty,
            clauses,
            &clause_params,
            arena_handle,
            cache_spec,
            timer_spec,
            struct_registry,
        )?;

        let needs_epilogue = prologue.needs_epilogue;

        if needs_epilogue {
            let ret_clif_ty = super::resolve_clif_ty(ret_ty, struct_registry);
            let epi = lower.builder.create_block();
            lower.builder.append_block_param(epi, ret_clif_ty);
            lower.epilogue_block = Some(epi);
        }

        // ── Body ──
        match &body_kind {
            BodyKind::Clauses => {
                if clauses.len() == 1 && all_patterns_are_ident(&clauses[0].patterns) {
                    let clause = &clauses[0];
                    lower_with_bindings(&clause.with_bindings, &mut lower)?;
                    lower.emitted_tail_call = false;
                    lower.emitted_terminator = false;
                    let result = lower_clause_body(clause, &mut lower, None)?;
                    if !lower.emitted_terminator && !lower.emitted_tail_call {
                        let result = coerce_return(result, ret_ty, &mut lower);
                        if needs_epilogue {
                            lower.builder.ins().jump(
                                lower
                                    .epilogue_block
                                    .expect("epilogue_block definido quando needs_epilogue"),
                                &[cranelift_codegen::ir::BlockArg::Value(result)],
                            );
                        } else {
                            emit_close_io_handles(&mut lower);
                            lower.builder.ins().return_(&[result]);
                        }
                    }
                } else {
                    lower_clause_chain(clauses, &clause_params, &mut lower)?;
                }
            }
            BodyKind::CallInner { .. } => {
                let inner_ref =
                    call_inner_ref.expect("call_inner_ref definido quando BodyKind::CallInner");
                let rt_val = lower
                    .rt
                    .unwrap_or_else(|| lower.builder.ins().iconst(I64, 0));
                let arena = lower
                    .fiber_arena
                    .or(lower.caller_arena)
                    .unwrap_or_else(|| lower.builder.ins().iconst(I64, 0));
                let mut inner_call_args = vec![rt_val, arena, box_ptr];
                inner_call_args.extend(clause_params.iter().copied());
                let inner_call = lower.builder.ins().call(inner_ref, &inner_call_args);
                let inner_result = lower.builder.inst_results(inner_call)[0];

                if needs_epilogue {
                    let result = coerce_return(inner_result, ret_ty, &mut lower);
                    lower.builder.ins().jump(
                        lower
                            .epilogue_block
                            .expect("epilogue_block definido quando needs_epilogue"),
                        &[cranelift_codegen::ir::BlockArg::Value(result)],
                    );
                } else {
                    let result = coerce_return(inner_result, ret_ty, &mut lower);
                    emit_close_io_handles(&mut lower);
                    lower.builder.ins().return_(&[result]);
                }
            }
        }

        // ── Epílogo ──
        if needs_epilogue {
            lower_epilogue(&mut lower, name, ret_ty, clauses, timer_spec, &prologue)?;
        }

        builder.finalize();
    }

    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("define fn {name}: {e}"),
        })?;
    if dump_ir {
        ir_dump.push((ir_name.to_string(), format!("{}", ctx.func.display())));
    }
    module.clear_context(&mut ctx);
    Ok(())
}

/// Define (compila o corpo de) uma função Kata nomeada.
///
/// Se `needs_split(func)` e há inner FuncId em `inner_table`, define duas
/// funções: o inner (body puro com TCO, sem intrínsecas, sem synthetic) e o
/// wrapper (prólogo/epílogo com intrínsecas + synthetic, `call inner`).
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
        // 1. Inner: body puro com TCO, sem intrínsecas, sem synthetic.
        //    kata_ids = symbol_table (wrapper) → non-tail self-calls vão ao wrapper (cache).
        //    inner_kata_ids = {key → inner_id} → tail self-calls vão ao inner (TCO).
        let inner_clauses: Vec<TypedLambdaClause> = func
            .clauses
            .iter()
            .map(|c| TypedLambdaClause {
                patterns: c.patterns.clone(),
                body: c.body.clone(),
                synthetic_pre: Vec::new(),
                synthetic_post: Vec::new(),
                guards: c.guards.clone(),
                with_bindings: c.with_bindings.clone(),
            })
            .collect();
        let mut inner_ids_map = HashMap::new();
        inner_ids_map.insert(key.clone(), inner_id);
        define_function_body(
            &func.name,
            &func.param_types,
            &func.ret_ty,
            &inner_clauses,
            &[],
            &None,
            &None,
            BodyKind::Clauses,
            false, // inner: TCO ativo
            inner_id,
            &func.name,
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

        // 2. Wrapper: prólogo (intrínsecas + synthetic) → call inner → epílogo.
        let wrapper_ir_name = format!("{}__wrapper", func.name);
        define_function_body(
            &func.name,
            &func.param_types,
            &func.ret_ty,
            &func.clauses,
            &[],
            &func.cache_spec,
            &func.timer_spec,
            BodyKind::CallInner { inner_id },
            true, // wrapper: nunca return_call
            func_id,
            &wrapper_ir_name,
            module,
            ffi_ids,
            symbol_table,
            &HashMap::new(), // wrapper não tem inner refs (não faz return_call)
            string_table,
            bytes_table,
            struct_registry,
            type_id_map,
            dump_ir,
            ir_dump,
        )?;
    } else {
        // ── Sem split ──
        let empty_inner: HashMap<FuncKey, cranelift_module::FuncId> = HashMap::new();
        define_function_body(
            &func.name,
            &func.param_types,
            &func.ret_ty,
            &func.clauses,
            &[],
            &func.cache_spec,
            &func.timer_spec,
            BodyKind::Clauses,
            false,
            func_id,
            &func.name,
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
