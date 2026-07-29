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
use kata_core::ty::{PrimTy, Ty};
use kata_inference::CacheSpec;
use kata_inference::{
    CaptureInfo, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedLogSpec,
};

use super::LowerCtx;
use super::backend::ModuleBackend;
use super::clause::{
    all_patterns_are_ident, bind_patterns_to_params, lower_clause_body, lower_clause_chain,
    lower_with_bindings,
};
use super::log::inject_log;
use super::module::{CodegenError, FuncKey, StringTable};
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
    cache_spec: &Option<CacheSpec>,
    func_id: cranelift_module::FuncId,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    kata_ids: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    struct_registry: &kata_core::StructRegistry,
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
            arc_vars: Vec::new(),
            struct_registry,
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

        // Se @log quando Enter, injeta antes do body (prólogo).
        if let Some(TypedLogSpec::Enter { .. }) = log {
            inject_log(
                log.as_ref().expect("log é Some: guardado pelo match Enter"),
                &mut lower,
            )?;
        }

        // Cria epilogue_block se @log Exit (para interceptar retornos).
        let mut needs_epilogue = matches!(log, Some(TypedLogSpec::Exit { .. }));

        // ── @cache: cache lookup no prólogo ──
        // Para funções anotadas com @cache{strategy: "LRU"}, serializa
        // os args, faz cache_lookup. Se hit (≠0), retorna direto.
        // Se miss, executa o body e faz cache_insert no epílogo.
        let cache_handle_val = if cache_spec.is_some() && !clause_params.is_empty() {
            let builder = &mut lower.builder;

            // fn_id: hash canônico de nome + param_types + body.
            // Diferencia bodies diferentes com mesma assinatura (REPL iter)
            // e overloads monomórficos.
            let fn_id = canonical_fn_id(name, param_types, clauses);
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
                .map(|ty| build_type_descriptor(ty, struct_registry))
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

                let written = builder
                    .ins()
                    .call(*serialize_fn, &[
                        param_i64,
                        desc_slot,
                        desc_len_val,
                        buf_ptr,
                        remaining,
                    ]);
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
                builder.ins().bitcast(ret_clif_ty, MemFlagsData::new(), lookup_result)
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

            // @cache: insert no epílogo.
            if let Some((handle_val, key_slot, key_len_val)) = &cache_handle_val {
                let insert_fn = lower
                    .ffi_refs
                    .get("kata_rt_cache_insert")
                    .expect("kata_rt_cache_insert registrado");
                // Bitcast result→I64 se necessário (Float é F64).
                let result_ty = lower.builder.func.dfg.value_type(result);
                let result_i64 = if result_ty != I64 {
                    lower.builder.ins().bitcast(I64, MemFlagsData::new(), result)
                } else {
                    result
                };
                lower
                    .builder
                    .ins()
                    .call(*insert_fn, &[*handle_val, *key_slot, *key_len_val, result_i64]);
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
    struct_registry: &kata_core::StructRegistry,
) -> Result<(), CodegenError> {
    define_function_body(
        &func.name,
        &func.param_types,
        &func.ret_ty,
        &func.clauses,
        &[], // funções nomeadas não têm capture
        &func.log,
        &func.cache_spec,
        func_id,
        module,
        ffi_ids,
        symbol_table,
        string_table,
        struct_registry,
    )
}

/// Computa um fn_id canônico a partir de nome + param_types + body.
///
/// O fn_id diferencia:
/// - Overloads monomórficos (mesmo nome, tipos diferentes)
/// - Bodies diferentes com mesma assinatura (REPL iter — body muda, fn_id muda)
/// - Funções diferentes com mesmo nome em programas diferentes
///
/// Serializa nome + tipos + body em uma string canônica (sem spans, sem ty)
/// e aplica FNV-1a para produzir um i64 estável.
fn canonical_fn_id(name: &str, param_types: &[Ty], clauses: &[TypedLambdaClause]) -> i64 {
    let mut buf = String::new();
    buf.push_str(name);
    buf.push('|');
    for ty in param_types {
        buf.push_str(&format!("{ty}"));
        buf.push(',');
    }
    buf.push('|');
    for clause in clauses {
        // Serializa patterns da cláusula
        for pat in &clause.patterns {
            buf.push_str(&format!("{:?}", pat.node));
            buf.push(',');
        }
        buf.push('|');
        // Serializa body canônico (sem spans)
        canonical_expr(&clause.body.node, &mut buf);
        buf.push(';');
    }
    // FNV-1a hash (usando u64, cast para i64 no final)
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in buf.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

/// Serializa um TypedExpr em string canônica (sem spans, sem ty).
fn canonical_expr(expr: &TypedExpr, buf: &mut String) {
    match &expr.kind {
        TypedExprKind::IntLit { text } => {
            buf.push_str("Int:");
            buf.push_str(text);
        }
        TypedExprKind::FloatLit { text } => {
            buf.push_str("Float:");
            buf.push_str(text);
        }
        TypedExprKind::TextLit { text } => {
            buf.push_str("Text:");
            buf.push_str(text);
        }
        TypedExprKind::Unit => buf.push_str("Unit"),
        TypedExprKind::Ident { name } => {
            buf.push_str("Ident:");
            buf.push_str(name);
        }
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            buf.push_str("Call(");
            canonical_expr(&callee.node, buf);
            buf.push(',');
            for arg in args {
                canonical_expr(&arg.node, buf);
                buf.push(',');
            }
            buf.push(')');
            if let Some(ffi) = ffi_symbol {
                buf.push_str(":ffi:");
                buf.push_str(ffi);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => {
            buf.push_str("Ascribe(");
            canonical_expr(&expr.node, buf);
            buf.push(')');
        }
        TypedExprKind::Grouping { inner } => {
            buf.push_str("Group(");
            canonical_expr(&inner.node, buf);
            buf.push(')');
        }
        TypedExprKind::Tuple { elements } => {
            buf.push_str("Tuple(");
            for el in elements {
                canonical_expr(&el.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::StructConstruct {
            struct_name,
            values,
        } => {
            buf.push_str("Struct(");
            buf.push_str(struct_name);
            buf.push(',');
            for v in values {
                canonical_expr(&v.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::FieldAccess {
            expr,
            struct_name,
            field_name,
            ..
        } => {
            buf.push_str("Field(");
            canonical_expr(&expr.node, buf);
            buf.push(',');
            buf.push_str(struct_name);
            buf.push(',');
            buf.push_str(field_name);
            buf.push(')');
        }
        TypedExprKind::IndexAccess { expr, index, .. } => {
            buf.push_str("Index(");
            canonical_expr(&expr.node, buf);
            buf.push(',');
            buf.push_str(&index.to_string());
            buf.push(')');
        }
        TypedExprKind::Let { name, value } => {
            buf.push_str("Let(");
            buf.push_str(name);
            buf.push(',');
            canonical_expr(&value.node, buf);
            buf.push(')');
        }
        TypedExprKind::LetDestruct {
            temp_name,
            value,
            bindings,
        } => {
            buf.push_str("LetDestruct(");
            buf.push_str(temp_name);
            buf.push(',');
            canonical_expr(&value.node, buf);
            buf.push(',');
            for (n, e) in bindings {
                buf.push_str(n);
                buf.push('=');
                canonical_expr(&e.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::VariantQual {
            enum_name, variant, ..
        } => {
            buf.push_str("Variant(");
            buf.push_str(enum_name);
            buf.push(',');
            buf.push_str(variant);
            buf.push(')');
        }
        TypedExprKind::VariantConstruct {
            enum_name,
            variant,
            payload,
            ..
        } => {
            buf.push_str("VariantC(");
            buf.push_str(enum_name);
            buf.push(',');
            buf.push_str(variant);
            buf.push(',');
            canonical_expr(&payload.node, buf);
            buf.push(')');
        }
        TypedExprKind::Lambda {
            func_name,
            param_types,
            ..
        } => {
            buf.push_str("Lambda(");
            if let Some(n) = func_name {
                buf.push_str(n);
            }
            buf.push(',');
            for ty in param_types {
                buf.push_str(&format!("{ty}"));
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::Return(expr) => {
            buf.push_str("Return(");
            canonical_expr(&expr.node, buf);
            buf.push(')');
        }
        TypedExprKind::Fork {
            action_name,
            action_expr,
            ..
        } => {
            buf.push_str("Fork(");
            buf.push_str(action_name);
            buf.push(',');
            canonical_expr(&action_expr.node, buf);
            buf.push(')');
        }
        TypedExprKind::ActionCall {
            callee,
            ffi_symbol,
            args,
            ..
        } => {
            buf.push_str("ActionCall(");
            buf.push_str(callee);
            if let Some(ffi) = ffi_symbol {
                buf.push_str(":ffi:");
                buf.push_str(ffi);
            }
            buf.push(',');
            canonical_expr(&args.node, buf);
            buf.push(')');
        }
        TypedExprKind::Match { scrutinee, arms } => {
            buf.push_str("Match(");
            canonical_expr(&scrutinee.node, buf);
            buf.push(',');
            for arm in arms {
                if let Some(p) = &arm.pattern {
                    buf.push_str(&format!("{:?}", p.node));
                } else {
                    buf.push_str("otherwise");
                }
                buf.push('=');
                canonical_expr(&arm.body.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::Comptime { expr } => {
            buf.push_str("Comptime(");
            canonical_expr(&expr.node, buf);
            buf.push(')');
        }
        // Catch-all para variants que não afetam o fn_id de funções @cache
        // (Int => Int): coleções, CSP, loops, type introspection, etc.
        // Estes nós não aparecem em funções puras Int => Int.
        _ => buf.push_str("Other"),
    }
}

// ── Type descriptor para cache key serialização ──────────────────────
//
// Construído em compile-time pelo codegen. Byte array C-ABI estável que
// descreve o layout do tipo para o runtime serializar o valor por conteúdo.
//
// Tags:
//   0x00 Unit     — 0 bytes
//   0x01 Int      — 8 bytes valor imediato
//   0x02 Float    — 8 bytes valor (f64 bits)
//   0x03 Text     — C string: len (4 bytes LE) + bytes
//   0x04 List(T)  — percorre cons cells, serializa cada head com T
//   0x05 Struct   — n_fields (u8) + field descriptors
//   0x06 Tuple    — n_elems (u8) + elem descriptors
//   0x07 Sum      — tag (8 bytes) + payload (8 bytes crus)
//                  Limitação: payload não serializado recursivamente sem
//                  enum_registry. Dois Sums com mesmo payload em endereços
//                  diferentes terão keys diferentes.

const TD_UNIT: u8 = 0x00;
const TD_INT: u8 = 0x01;
const TD_FLOAT: u8 = 0x02;
const TD_TEXT: u8 = 0x03;
const TD_LIST: u8 = 0x04;
const TD_STRUCT: u8 = 0x05;
const TD_TUPLE: u8 = 0x06;
const TD_SUM: u8 = 0x07;

/// Constrói um type descriptor (byte array) para um `Ty`.
fn build_type_descriptor(ty: &Ty, struct_registry: &kata_core::StructRegistry) -> Vec<u8> {
    let mut buf = Vec::new();
    write_descriptor(&mut buf, ty, struct_registry);
    buf
}

fn write_descriptor(buf: &mut Vec<u8>, ty: &Ty, struct_registry: &kata_core::StructRegistry) {
    match ty {
        Ty::Unit => buf.push(TD_UNIT),
        Ty::Prim(PrimTy::Int) => buf.push(TD_INT),
        Ty::Prim(PrimTy::Float) => buf.push(TD_FLOAT),
        Ty::Prim(PrimTy::Text) => buf.push(TD_TEXT),
        Ty::Prim(_) => buf.push(TD_INT),
        Ty::List(elem) => {
            buf.push(TD_LIST);
            write_descriptor(buf, elem, struct_registry);
        }
        Ty::Tuple(elements) => {
            buf.push(TD_TUPLE);
            buf.push(elements.len() as u8);
            for elem in elements {
                write_descriptor(buf, elem, struct_registry);
            }
        }
        Ty::Struct(name) => {
            buf.push(TD_STRUCT);
            if let Some(info) = struct_registry.get(name) {
                buf.push(info.fields.len() as u8);
                for field in &info.fields {
                    write_descriptor(buf, &field.ty, struct_registry);
                }
            } else {
                buf.push(1);
                buf.push(TD_INT);
            }
        }
        Ty::Sum(_) | Ty::Generic(_, _) => {
            buf.push(TD_SUM);
        }
        _ => {
            buf.push(TD_INT);
        }
    }
}
