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

use std::collections::HashMap;

use crate::call_conv::ffi_call_conv;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, GlobalValueData, InstBuilder, Signature};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::{TypedAction, TypedTestSpec};
use kata_resolution::MatchPolicy;

use super::LowerCtx;
use super::backend::ModuleBackend;
use super::expr::lower_expr;
use super::module::{CodegenError, FuncKey, StringTable};
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
/// Retorna a lista de wrappers gerados. Chamado por `lower_module` após
/// declarar e definir Actions (para que `symbol_table` tenha os FuncIds).
#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_test_wrappers(
    typed: &kata_inference::TypedModule,
    module: &mut dyn ModuleBackend,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &HashMap<FuncKey, cranelift_module::FuncId>,
    string_table: &mut StringTable,
    bytes_table: &mut Vec<Vec<u8>>,
    fn_counter: &mut u64,
    struct_registry: &kata_core::StructRegistry,
    type_id_map: &HashMap<Ty, i64>,
    dump_ir: bool,
    ir_dump: &mut Vec<(String, String)>,
) -> Result<Vec<TestWrapper>, CodegenError> {
    let mut wrappers = Vec::new();

    for action in &typed.actions {
        for (test_index, spec) in action.tests.iter().enumerate() {
            // Validação: action com params exige args no @test. Sem args,
            // o wrapper passa args_ptr = 0 (null) e a action lê params de
            // null → SIGSEGV em runtime. Falhar aqui com erro claro.
            if !action.param_types.is_empty() && spec.args.is_none() {
                let params = action
                    .param_types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(CodegenError::UnsupportedNode {
                    node: format!(
                        "@test sem args em action `{}` que recebe ({}) — \
                     forneça args: (..) no @test",
                        action.name, params
                    ),
                });
            }

            let cranelift_name = format!("__kata_fn_{}", *fn_counter);
            *fn_counter += 1;

            let func_id = declare_test_wrapper(&cranelift_name, module)?;
            let mut tctx = TestLowerCtx {
                module,
                ffi_ids,
                symbol_table,
                string_table: &mut *string_table,
                bytes_table: &mut *bytes_table,
                struct_registry,
                type_id_map,
                dump_ir,
                ir_dump: &mut *ir_dump,
            };
            define_test_wrapper(action, spec, func_id, &mut tctx)?;

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

/// Declara um wrapper `(rt: i64) -> i64` com `CallConv::SystemV`.
///
/// A2: rt é ponteiro para Box<Runtime>, passado pelo driver.
fn declare_test_wrapper(
    cranelift_name: &str,
    module: &mut dyn ModuleBackend,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(ffi_call_conv());
    sig.params.push(AbiParam::new(I64)); // rt
    sig.returns.push(AbiParam::new(I64));
    module
        .declare_function(cranelift_name, Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("declare test wrapper: {e}"),
        })
}

/// Contexto de lowering compartilhado entre wrappers de teste.
///
/// Agrupa as tabelas e backend que `define_test_wrapper` precisa acessar.
/// Evita passar 5 parâmetros isolados — clippy::too_many_arguments.
pub(crate) struct TestLowerCtx<'a> {
    pub module: &'a mut dyn ModuleBackend,
    pub ffi_ids: &'a HashMap<String, cranelift_module::FuncId>,
    pub symbol_table: &'a HashMap<FuncKey, cranelift_module::FuncId>,
    pub string_table: &'a mut StringTable,
    pub bytes_table: &'a mut Vec<Vec<u8>>,
    pub struct_registry: &'a kata_core::StructRegistry,
    pub type_id_map: &'a HashMap<Ty, i64>,
    pub dump_ir: bool,
    pub ir_dump: &'a mut Vec<(String, String)>,
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
    tctx: &mut TestLowerCtx,
) -> Result<(), CodegenError> {
    let mut ctx = tctx.module.make_context();
    let mut metadata = MetadataTable::new();

    {
        let func_ir = &mut ctx.func;
        let mut sig = Signature::new(ffi_call_conv());
        sig.params.push(AbiParam::new(I64)); // rt
        sig.returns.push(AbiParam::new(I64));
        func_ir.signature = sig;

        // Declara FFI e funções Kata no Function.
        let mut ffi_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (fname, &fid) in tctx.ffi_ids {
            let func_ref = tctx.module.declare_func_in_func(fid, func_ir);
            ffi_refs.insert(fname.clone(), func_ref);
        }
        let mut kata_refs: HashMap<FuncKey, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (key, &fid) in tctx.symbol_table {
            let func_ref = tctx.module.declare_func_in_func(fid, func_ir);
            kata_refs.insert(key.clone(), func_ref);
        }

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(func_ir, &mut func_ctx);

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        // A2: rt é o primeiro (e único) block param — ponteiro para Box<Runtime>.
        let rt_value = builder.block_params(entry_block)[0];

        let mut lower = LowerCtx {
            builder: &mut builder,
            module: tctx.module,
            ffi_refs: &ffi_refs,
            kata_refs: &kata_refs,
            ffi_ids: tctx.ffi_ids,
            kata_ids: tctx.symbol_table,
            metadata: &mut metadata,
            string_table: tctx.string_table,
            bytes_table: tctx.bytes_table,
            var_map: HashMap::new(),
            anon_counter: 0,
            emitted_tail_call: false,
            emitted_terminator: false,
            no_tail_calls: true, // SystemV — sem return_call
            epilogue_block: None,
            fiber_arena: None,
            caller_arena: None,
            scheduler_mode: true, // wrapper usa spawn+run como o entry point
            loop_break_block: None,
            loop_continue_block: None,
            io_handle_vars: Vec::new(),
            struct_registry: tctx.struct_registry,
            type_id_map: tctx.type_id_map,
            ipc_broker_fid: None,
            rt: None,
            dump_ir: tctx.dump_ir,
            ir_dump: &mut *tctx.ir_dump,
        };

        // 1. scheduler_init(rt) → root_arena (igual ao entry point).
        let scheduler_init_ref = lower
            .ffi_refs
            .get("kata_rt_scheduler_init")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_scheduler_init".into(),
            })?;
        let init_inst = lower.builder.ins().call(scheduler_init_ref, &[rt_value]);
        let root_arena = lower.builder.inst_results(init_inst)[0];
        lower.caller_arena = Some(root_arena);
        lower.rt = Some(rt_value);

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
        let callee_fid =
            *lower
                .kata_ids
                .get(&action_key)
                .ok_or_else(|| CodegenError::UnsupportedNode {
                    node: format!(
                        "test wrapper: Action `{}` não encontrada em symbol_table",
                        action.name
                    ),
                })?;
        let func_ref = lower
            .module
            .declare_func_in_func(callee_fid, lower.builder.func);
        let ext_func_name = lower.builder.func.dfg.ext_funcs[func_ref].name.clone();
        let func_gv = lower
            .builder
            .func
            .create_global_value(GlobalValueData::Symbol {
                name: ext_func_name,
                offset: 0.into(),
                colocated: true,
                tls: false,
            });
        let fn_ptr = lower
            .builder
            .ins()
            .global_value(lower.module.target_config().pointer_type(), func_gv);

        // 4. kata_rt_spawn(rt, fn_ptr, root_arena, args_ptr) → fiber_id
        let spawn_ref = lower
            .ffi_refs
            .get("kata_rt_spawn")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_spawn".into(),
            })?;
        lower
            .builder
            .ins()
            .call(spawn_ref, &[rt_value, fn_ptr, root_arena, args_ptr]);

        // 5. kata_rt_run(rt) → result (i64)
        let run_ref = lower.ffi_refs.get("kata_rt_run").copied().ok_or_else(|| {
            CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_run".into(),
            }
        })?;
        let run_inst = lower.builder.ins().call(run_ref, &[rt_value]);
        let result = lower.builder.inst_results(run_inst)[0];

        // 6. Retorno — verifica expects se presente, senão retorna resultado bruto.
        let ret_val = if action.ret_ty == Ty::float() {
            lower
                .builder
                .ins()
                .bitcast(I64, cranelift_codegen::ir::MemFlagsData::new(), result)
        } else {
            result
        };

        if let Some(expects_str) = spec.expects.as_ref() {
            // ── Verificação de expects ──
            // O wrapper retorna status codes:
            //   0 = pass (show(err) casou expects com policy)
            //   1 = fail (show(err) não casou)
            //   2 = fail (action retornou Ok quando esperava Err)
            // Sentinel values (timeout/deadlock) são repassados intactos.

            // Blocks para os branches.
            let sentinel_block = lower.builder.create_block();
            let check_tag_block = lower.builder.create_block();
            let ok_block = lower.builder.create_block();
            let err_block = lower.builder.create_block();
            let pass_block = lower.builder.create_block();
            let fail_block = lower.builder.create_block();

            // Checar se result é sentinel (timeout/deadlock).
            // TIMEOUT_SENTINEL = i64::MIN + 2, DEADLOCK_SENTINEL = i64::MIN + 1.
            // Se result <= i64::MIN + 2, é sentinel — retornar result intacto.
            let sentinel_threshold = lower.builder.ins().iconst(I64, i64::MIN + 2);
            let is_sentinel = lower.builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::SignedLessThanOrEqual,
                result,
                sentinel_threshold,
            );
            lower
                .builder
                .ins()
                .brif(is_sentinel, sentinel_block, &[], check_tag_block, &[]);

            // sentinel_block: retornar result (timeout/deadlock).
            lower.builder.switch_to_block(sentinel_block);
            lower.builder.seal_block(sentinel_block);
            lower.builder.ins().return_(&[result]);

            // check_tag_block: extrair tag do Sum.
            lower.builder.switch_to_block(check_tag_block);
            lower.builder.seal_block(check_tag_block);
            let tag_ref = lower
                .ffi_refs
                .get("kata_rt_sum_tag_int")
                .copied()
                .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                    symbol: "kata_rt_sum_tag_int".into(),
                })?;
            let tag_inst = lower.builder.ins().call(tag_ref, &[result]);
            let tag = lower.builder.inst_results(tag_inst)[0];
            let zero = lower.builder.ins().iconst(I64, 0);
            let is_ok =
                lower
                    .builder
                    .ins()
                    .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, tag, zero);
            lower
                .builder
                .ins()
                .brif(is_ok, ok_block, &[], err_block, &[]);

            // ok_block: action retornou Ok — expected Err.
            lower.builder.switch_to_block(ok_block);
            lower.builder.seal_block(ok_block);
            let two = lower.builder.ins().iconst(I64, 2);
            lower.builder.ins().return_(&[two]);

            // err_block: extrair payload, chamar show, comparar com expects.
            lower.builder.switch_to_block(err_block);
            lower.builder.seal_block(err_block);

            // Load payload do offset 8 do Sum box.
            let payload = lower.builder.ins().load(
                I64,
                cranelift_codegen::ir::MemFlagsData::new(),
                result,
                8,
            );

            // Determinar o tipo do payload de Err e chamar show apropriado.
            // ret_ty é Ty::Generic("Result", [T, E]).
            // E é o segundo type arg — o tipo do payload de Err.
            let shown = lower_expects_show(&action.ret_ty, payload, &mut lower)?;

            // Carregar string expects como global data.
            let expects_global = lower.add_string(expects_str);
            let expects_ptr = lower
                .builder
                .ins()
                .global_value(lower.module.target_config().pointer_type(), expects_global);

            // Chamar FFI de comparação conforme policy.
            let cmp_fn_name = match spec.policy.unwrap_or(MatchPolicy::Exact) {
                MatchPolicy::Exact => "kata_rt_string_eq",
                MatchPolicy::Prefix => "kata_rt_string_starts_with",
                MatchPolicy::Contains => "kata_rt_string_contains",
            };
            let cmp_ref = lower.ffi_refs.get(cmp_fn_name).copied().ok_or_else(|| {
                CodegenError::FfiSymbolNotFound {
                    symbol: cmp_fn_name.into(),
                }
            })?;
            let cmp_inst = lower.builder.ins().call(cmp_ref, &[shown, expects_ptr]);
            let cmp_result = lower.builder.inst_results(cmp_inst)[0];
            let one = lower.builder.ins().iconst(I64, 1);
            let is_match = lower.builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::Equal,
                cmp_result,
                one,
            );
            lower
                .builder
                .ins()
                .brif(is_match, pass_block, &[], fail_block, &[]);

            // pass_block: return 0.
            lower.builder.switch_to_block(pass_block);
            lower.builder.seal_block(pass_block);
            let zero_status = lower.builder.ins().iconst(I64, 0);
            lower.builder.ins().return_(&[zero_status]);

            // fail_block: return 1.
            lower.builder.switch_to_block(fail_block);
            lower.builder.seal_block(fail_block);
            let one_status = lower.builder.ins().iconst(I64, 1);
            lower.builder.ins().return_(&[one_status]);
        } else {
            // Sem expects — retornar resultado bruto (comportamento atual).
            lower.builder.ins().return_(&[ret_val]);
        }

        builder.finalize();
    }

    tctx.module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift {
            reason: format!("define test wrapper: {e}"),
        })?;
    if tctx.dump_ir {
        tctx.ir_dump.push((
            format!("__kata_test_{}_{}", action.name, spec.desc.as_deref().unwrap_or("")),
            format!("{}", ctx.func.display()),
        ));
    }
    tctx.module.clear_context(&mut ctx);
    Ok(())
}

/// Chama `show` no payload de `Result::Err`, retornando um `i64` (ponteiro C string).
///
/// `ret_ty` é o tipo de retorno da action — esperado `Ty::Generic("Result", [T, E])`.
/// Extrai `E` (segundo type arg) para determinar qual `show` chamar:
///
/// - `Ty::Sum(name)` → chama `__kata_show__{name}` via kata_refs (enum não-genérico).
/// - `Ty::Prim(PrimTy::Text)` → chama `kata_rt_bi_show` via ffi_refs (Text direto).
/// - `Ty::Prim(PrimTy::Int)` → chama `kata_rt_bi_show` via ffi_refs.
/// - Outros → fallback gracoso: chama `kata_rt_int_to_text` (representação genérica).
fn lower_expects_show(
    ret_ty: &Ty,
    payload: cranelift_codegen::ir::Value,
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    use kata_core::ty::PrimTy;

    // Extrair E de Ty::Generic("Result", [T, E]).
    let err_ty = match ret_ty {
        Ty::Generic(name, args) if name == "Result" && args.len() >= 2 => &args[1],
        _ => {
            // Não é Result — fallback: int_to_text no payload.
            let ffi = lower
                .ffi_refs
                .get("kata_rt_int_to_text")
                .copied()
                .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                    symbol: "kata_rt_int_to_text".into(),
                })?;
            let inst = lower.builder.ins().call(ffi, &[payload]);
            return Ok(lower.builder.inst_results(inst)[0]);
        }
    };

    match err_ty {
        Ty::Sum(enum_name) => {
            // Enum não-genérico: chamar __kata_show__{enum_name} via kata_refs.
            let show_name = format!("__kata_show__{enum_name}");
            let show_key: FuncKey = (show_name, vec![Ty::Sum(enum_name.clone())], Ty::text());
            let show_fid = lower.kata_ids.get(&show_key).copied().ok_or_else(|| {
                CodegenError::UnsupportedNode {
                    node: format!("show para enum `{enum_name}` não encontrado na symbol_table"),
                }
            })?;
            let show_ref = lower
                .module
                .declare_func_in_func(show_fid, lower.builder.func);
            // ABI Kata: (rt, arena_handle, box_ptr, payload) -> i64.
            let rt_val = lower
                .rt
                .unwrap_or_else(|| lower.builder.ins().iconst(I64, 0));
            let arena = lower
                .caller_arena
                .unwrap_or_else(|| lower.builder.ins().iconst(I64, 0));
            let dummy_box = lower.builder.ins().iconst(I64, 0);
            let inst = lower
                .builder
                .ins()
                .call(show_ref, &[rt_val, arena, dummy_box, payload]);
            Ok(lower.builder.inst_results(inst)[0])
        }
        Ty::Prim(PrimTy::Text) => {
            // Text direto: o payload já é um ponteiro C string.
            // show de Text cita com aspas: "\"text\"". Usar kata_rt_bi_show.
            let ffi = lower
                .ffi_refs
                .get("kata_rt_bi_show")
                .copied()
                .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                    symbol: "kata_rt_bi_show".into(),
                })?;
            let inst = lower.builder.ins().call(ffi, &[payload]);
            Ok(lower.builder.inst_results(inst)[0])
        }
        Ty::Prim(PrimTy::Int) => {
            let ffi = lower
                .ffi_refs
                .get("kata_rt_bi_show")
                .copied()
                .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                    symbol: "kata_rt_bi_show".into(),
                })?;
            let inst = lower.builder.ins().call(ffi, &[payload]);
            Ok(lower.builder.inst_results(inst)[0])
        }
        _ => {
            // Fallback gracoso para outros tipos.
            let ffi = lower
                .ffi_refs
                .get("kata_rt_int_to_text")
                .copied()
                .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                    symbol: "kata_rt_int_to_text".into(),
                })?;
            let inst = lower.builder.ins().call(ffi, &[payload]);
            Ok(lower.builder.inst_results(inst)[0])
        }
    }
}
