//! Codegen de `@log` — injeta `kata_rt_log_publish` no prólogo ou epílogo.
//!
//! Chamado por `action_def.rs` e `function_def.rs` quando `TypedAction.log`
//! ou `TypedFunction.log` é `Some`.

use cranelift_codegen::ir::InstBuilder;
use cranelift_codegen::ir::types::I64;
use cranelift_module::Module;
use kata_inference::TypedLogSpec;

use super::LowerCtx;
use super::expr::lower_expr;

/// Lowera a expressão da mensagem e injeta `kata_rt_log_publish`.
///
/// `topic`/`policy` None → passa 0 (runtime usa config herdada).
/// Retorna o valor SSA do status (i64) — tipicamente descartado.
pub(crate) fn inject_log(
    log: &TypedLogSpec,
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let (msg_expr, topic, policy, level) = match log {
        TypedLogSpec::Enter {
            msg_expr,
            topic,
            policy,
            level,
        } => (msg_expr, topic, policy, level),
        TypedLogSpec::Exit {
            msg_expr,
            topic,
            policy,
            level,
        } => (msg_expr, topic, policy, level),
    };

    // Lowera a expressão da mensagem (produz handle Text — ponteiro para string).
    let msg_ptr = lower_expr(&msg_expr.node, lower)?;

    // topic_ptr: se Some, aloca string como data symbol; se None, 0.
    let topic_ptr = if let Some(t) = topic {
        let global = lower.add_string(t);
        lower
            .builder
            .ins()
            .global_value(lower.module.target_config().pointer_type(), global)
    } else {
        lower.builder.ins().iconst(I64, 0)
    };

    // policy_ptr: idem.
    let policy_ptr = if let Some(p) = policy {
        let global = lower.add_string(p);
        lower
            .builder
            .ins()
            .global_value(lower.module.target_config().pointer_type(), global)
    } else {
        lower.builder.ins().iconst(I64, 0)
    };

    // level: tag do enum LogLevel (i64).
    let level_val = lower.builder.ins().iconst(I64, *level);

    // Chama kata_rt_log_publish(topic_ptr, level, msg_ptr, policy_ptr).
    let fref = lower
        .ffi_refs
        .get("kata_rt_log_publish")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound {
            symbol: "kata_rt_log_publish".into(),
        })?;
    let call = lower
        .builder
        .ins()
        .call(fref, &[topic_ptr, level_val, msg_ptr, policy_ptr]);
    Ok(lower.builder.inst_results(call)[0])
}
