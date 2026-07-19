//! Wrappers de teste `__kata_test_*` — um por `@test` descoberto.
//!
//! Cada wrapper é uma função JIT `() -> i64` com `CallConv::SystemV` que:
//! 1. Inicializa o scheduler (`kata_rt_scheduler_init` → `root_arena`).
//! 2. Lowera os args literais do `@test` (tupla → `args_ptr`, ou 0 se Unit).
//! 3. Obtém o `fn_ptr` da Action via `GlobalValue::Symbol`.
//! 4. Chama `kata_rt_spawn(fn_ptr, root_arena, args_ptr)`.
//! 5. Chama `kata_rt_run()` → resultado (i64).
//! 6. Retorna o resultado.
//!
//! O runner (driver) faz `reset_scheduler` + `kata_rt_set_test_timeout(N)` +
//! chama o wrapper. O wrapper é autossuficiente — não acopla o driver ao
//! ABI interno das Actions.
//!
//! Testes negativos (`expects: "CompileError: ..."`) NÃO geram wrapper —
//! o codegen pula a geração. O driver tenta compilar o sub-módulo isolado.

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, GlobalValueData, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::{TypedAction, TypedTestSpec};

use super::expr::lower_expr;
use super::module::{CodegenError, FuncKey, StringTable};
use super::LowerCtx;
use crate::metadata::MetadataTable;

/// Identidade semântica de um wrapper de teste — tupla, não string fabricada.
/// `(action_name, test_index)` onde `test_index` é posicional dentro de
/// `typed_action.tests`. O `FuncId` é o plumbing no JITModule.
#[derive(Debug, Clone)]
pub struct TestWrapper {
    pub action_name: String,
    pub test_index: usize,
    pub func_id: cranelift_module::FuncId,
    pub spec: TypedTestSpec,
}

/// Gera wrappers `__kata_test_*` para todos os `@test` não-negativos.
///
/// Negativos (`expects: "CompileError: ..."`) são pulados — o driver
/// compila o sub-módulo isolado em vez de executar um wrapper.
///
/// Retorna a lista de wrappers gerados. Chamado por `lower_module` após
/// declarar e definir Actions (para que `symbol_table` tenha os FuncIds).
pub(crate) fn generate_test_wrappers(
    typed: &kata_inference::TypedModule,
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    fn_counter: &mut u64,
) -> Result<Vec<TestWrapper>, CodegenError> {
    let mut wrappers = Vec::new();

    for action in &typed.actions {
        for (test_index, spec) in action.tests.iter().enumerate() {
            // Negativos (CompileError) não geram wrapper.
            if let Some(expects) = &spec.expects
                && expects.starts_with("CompileError:")
            {
                wrappers.push(TestWrapper {
                    action_name: action.name.clone(),
                    test_index,
                    func_id: cranelift_module::FuncId::from_u32(0), // placeholder
                    spec: spec.clone(),
                });
                continue;
            }

            let cranelift_name = format!("__kata_fn_{}", *fn_counter);
            *fn_counter += 1;

            let func_id = declare_test_wrapper(&cranelift_name, module)?;
            define_test_wrapper(
                action,
                spec,
                func_id,
                module,
                ffi_ids,
                symbol_table,
                &mut *string_table,
            )?;

            wrappers.push(TestWrapper {
                action_name: action.name.clone(),
                test_index,
                func_id,
                spec: spec.clone(),
            });
        }
    }

    Ok(wrappers)
}

/// Declara um wrapper `() -> i64` com `CallConv::SystemV`.
fn declare_test_wrapper(
    cranelift_name: &str,
    module: &mut cranelift_jit::JITModule,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.returns.push(AbiParam::new(I64));
    module
        .declare_function(cranelift_name, Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare test wrapper: {e}")))
}

/// Define (compila) o corpo de um wrapper de teste.
///
/// Corpo:
/// 1. `scheduler_init` → `root_arena`
/// 2. Lowera args (se `Some`) → `args_ptr`; senão `iconst(0)` (Unit)
/// 3. `GlobalValue::Symbol` da Action → `fn_ptr`
/// 4. `kata_rt_spawn(fn_ptr, root_arena, args_ptr)`
/// 5. `kata_rt_run()` → `result`
/// 6. `return_(result)`
fn define_test_wrapper(
    action: &TypedAction,
    spec: &TypedTestSpec,
    func_id: cranelift_module::FuncId,
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        let mut sig = Signature::new(CallConv::SystemV);
        sig.returns.push(AbiParam::new(I64));
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
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

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
            no_tail_calls: true, // SystemV — sem return_call
            epilogue_block: None,
            fiber_arena: None,
            caller_arena: None,
            scheduler_mode: true, // wrapper usa spawn+run como o entry point
            loop_break_block: None,
            loop_continue_block: None,
            closure_captures: HashMap::new(),
        };

        // 1. scheduler_init → root_arena (igual ao entry point).
        let scheduler_init_ref = lower
            .ffi_refs
            .get("kata_rt_scheduler_init")
            .copied()
            .ok_or_else(|| {
                CodegenError::FfiSymbolNotFound("kata_rt_scheduler_init".into())
            })?;
        let init_inst = lower.builder.ins().call(scheduler_init_ref, &[]);
        let root_arena = lower.builder.inst_results(init_inst)[0];
        lower.caller_arena = Some(root_arena);

        // 2. Lowera args do @test → args_ptr.
        // Se args é None, passa 0 (Unit). Se é Some, lowera o TypedExpr
        // (que produz um ponteiro para tupla na arena).
        let args_ptr = if let Some(args_expr) = &spec.args {
            lower_expr(&args_expr.node, &mut lower)?
        } else {
            lower.builder.ins().iconst(I64, 0)
        };

        // 3. Obter fn_ptr da Action via GlobalValue::Symbol.
        let action_key: FuncKey = (
            action.name.clone(),
            action.param_types.clone(),
            action.ret_ty.clone(),
        );
        let callee_fid = *lower.kata_ids.get(&action_key).ok_or_else(|| {
            CodegenError::UnsupportedNode(format!(
                "test wrapper: Action `{}` não encontrada em symbol_table",
                action.name
            ))
        })?;
        let func_ref = lower
            .module
            .declare_func_in_func(callee_fid, lower.builder.func);
        let ext_func_name = lower.builder.func.dfg.ext_funcs[func_ref].name.clone();
        let func_gv = lower.builder.func.create_global_value(GlobalValueData::Symbol {
            name: ext_func_name,
            offset: 0.into(),
            colocated: true,
            tls: false,
        });
        let fn_ptr = lower
            .builder
            .ins()
            .global_value(lower.module.target_config().pointer_type(), func_gv);

        // 4. kata_rt_spawn(fn_ptr, root_arena, args_ptr) → fiber_id
        let spawn_ref = lower
            .ffi_refs
            .get("kata_rt_spawn")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_spawn".into()))?;
        lower
            .builder
            .ins()
            .call(spawn_ref, &[fn_ptr, root_arena, args_ptr]);

        // 5. kata_rt_run() → result (i64)
        let run_ref = lower
            .ffi_refs
            .get("kata_rt_run")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_run".into()))?;
        let run_inst = lower.builder.ins().call(run_ref, &[]);
        let result = lower.builder.inst_results(run_inst)[0];

        // 6. return_(result) — Float bitcast se necessário.
        let ret_val = if action.ret_ty == Ty::float() {
            lower
                .builder
                .ins()
                .bitcast(I64, cranelift_codegen::ir::MemFlagsData::new(), result)
        } else {
            result
        };
        lower.builder.ins().return_(&[ret_val]);

        builder.finalize();
    }

    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift(format!("define test wrapper: {e}")))?;
    module.clear_context(&mut ctx);
    Ok(())
}