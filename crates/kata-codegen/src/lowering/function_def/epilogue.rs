//! Epílogo de funções Kata: cache_insert → timer_stop → synthetic_post → return.
//!
//! Extraído de `function_def` para separar a fase de cleanup/retorno do prólogo.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};
use kata_core::ty::Ty;
use kata_inference::{TimerSpec, TypedLambdaClause};

use super::super::IoHandleKind;
use super::super::LowerCtx;
use super::super::expr::lower_expr;
use super::super::module::CodegenError;
use super::super::timer::inject_timer_stop;

use super::{PrologueResult, coerce_return};

/// Lowera o epílogo: cache_insert → timer_stop → synthetic_post → return.
pub(super) fn lower_epilogue(
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
        let ret_clif_ty = super::super::resolve_clif_ty(ret_ty, lower.struct_registry);
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
    lower.emit_depth_dec();
    lower.builder.ins().return_(&[result]);
    Ok(())
}

/// Emite close para cada variável em `io_handle_vars` no epílogo de uma
/// função. I/O handles não fechados explicitamente pelo programador são
/// fechados automaticamente antes do return.
pub(super) fn emit_close_io_handles(lower: &mut LowerCtx) {
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
            IoHandleKind::File => {
                lower.builder.ins().call(file_close_ref, &[val]);
            }
            IoHandleKind::Socket => {
                lower.builder.ins().call(socket_close_ref, &[val]);
            }
        }
    }
}
