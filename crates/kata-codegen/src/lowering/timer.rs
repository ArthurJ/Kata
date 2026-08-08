//! Codegen de `@timer` — injeta `kata_rt_timer_now()` no prólogo (start)
//! e computa delta + publica via `kata_rt_log_publish` no epílogo.
//!
//! Chamado por `function_def.rs` quando `TypedFunction.timer_spec` é `Some`.
//! Versão inicial: caso não-TCO (stack slot) — o start é armazenado num
//! stack slot do frame. O caso TCO (canal interno buffer-1) é follow-up.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, StackSlotData, StackSlotKind};
use cranelift_module::Module;
use kata_inference::TimerSpec;

use super::LowerCtx;
use super::CodegenError;

/// Aloca um stack slot para o timestamp de start e chama `kata_rt_timer_now()`.
/// Retorna o valor SSA do start (para armazenar no stack slot via caller).
pub(crate) fn inject_timer_start(
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    // Chama kata_rt_timer_now() → i64 (SMI-tagged nanossegundos).
    let now_fn = lower
        .ffi_refs
        .get("kata_rt_timer_now")
        .copied()
        .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_timer_now".into()))?;
    let call = lower.builder.ins().call(now_fn, &[]);
    let start_val = lower.builder.inst_results(call)[0];

    // Armazena start num stack slot para sobreviver ao body.
    let sslot = lower
        .builder
        .func
        .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 8));
    let slot_addr = lower.builder.ins().stack_addr(I64, sslot, 0);
    lower
        .builder
        .ins()
        .store(cranelift_codegen::ir::MemFlagsData::new(), start_val, slot_addr, 0);

    Ok(start_val)
}

/// No epílogo: carrega start do stack slot, chama `kata_rt_timer_now()` para
/// obter end, computa delta = end - start, formata mensagem e publica via
/// `kata_rt_log_publish`.
///
/// `start_val` é o valor SSA do start (do stack slot). `func_name` é usado
/// como default do tópico e na mensagem.
pub(crate) fn inject_timer_stop(
    timer: &TimerSpec,
    func_name: &str,
    start_val: cranelift_codegen::ir::Value,
    lower: &mut LowerCtx,
) -> Result<(), CodegenError> {
    // Recarrega start do stack slot (pode ter sido spilling).
    // start_val ainda é válido se o builder não o spilling, mas para garantir
    // que o valor está disponível após o body, usamos o valor SSA diretamente.
    let start = start_val;

    // Chama kata_rt_timer_now() para obter end.
    let now_fn = lower
        .ffi_refs
        .get("kata_rt_timer_now")
        .copied()
        .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_timer_now".into()))?;
    let call = lower.builder.ins().call(now_fn, &[]);
    let end = lower.builder.inst_results(call)[0];

    // delta = end - start via FFI (preserva SMI tagging).
    // isub cru quebra SMI: (a<<1|1) - (b<<1|1) = (a-b)<<1 (LSB=0, não SMI).
    // kata_rt_bi_sub decodifica, subtrai, re-codifica.
    let bi_sub_fn = lower
        .ffi_refs
        .get("kata_rt_bi_sub")
        .copied()
        .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_bi_sub".into()))?;
    let call = lower.builder.ins().call(bi_sub_fn, &[end, start]);
    let delta = lower.builder.inst_results(call)[0];

    // Formata a mensagem: "{name}: {delta}ns" (default sem stats).
    // Usa kata_rt_int_to_text para converter delta para Text,
    // e kata_rt_text_replace_first para substituir placeholders.
    let msg_template = timer
        .msg
        .as_deref()
        .unwrap_or("{name}: {delta}ns");

    // Constrói a mensagem via text_replace_first:
    // 1. Substitui {name} pelo nome da função.
    // 2. Substitui {delta} pelo delta formatado.
    let msg = format_timer_msg(msg_template, func_name, delta, lower)?;

    // topic: se Some, aloca string; se None, usa 0 (runtime usa default).
    let topic_ptr = if let Some(t) = &timer.topic {
        let global = lower.add_string(t);
        lower
            .builder
            .ins()
            .global_value(lower.module.target_config().pointer_type(), global)
    } else {
        // Default: nome da função como tópico.
        let global = lower.add_string(func_name);
        lower
            .builder
            .ins()
            .global_value(lower.module.target_config().pointer_type(), global)
    };

    // level: Info (tag 1).
    let level_val = lower.builder.ins().iconst(I64, 1);

    // policy: 0 (default — drop).
    let policy_ptr = lower.builder.ins().iconst(I64, 0);

    // Chama kata_rt_log_publish(topic_ptr, level, msg_ptr, policy_ptr).
    let publish_fn = lower
        .ffi_refs
        .get("kata_rt_log_publish")
        .copied()
        .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_log_publish".into()))?;
    lower
        .builder
        .ins()
        .call(publish_fn, &[topic_ptr, level_val, msg, policy_ptr]);

    Ok(())
}

/// Formata a mensagem do timer substituindo `{name}` e `{delta}`.
///
/// Usa `kata_rt_text_replace_first` (que substitui a primeira ocorrência de
/// `{}` pelo replacement) e `kata_rt_int_to_text` para converter o delta.
///
/// A mensagem default é `"{name}: {delta}ns"`. O codegen constrói:
/// 1. Template com `{}` no lugar de `{name}` e `{delta}`: `"{}: {}ns"`
/// 2. Primeiro `{}` → nome da função (string literal)
/// 3. Segundo `{}` → delta em Text (via `kata_rt_int_to_text`)
fn format_timer_msg(
    template: &str,
    func_name: &str,
    delta: cranelift_codegen::ir::Value,
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    // Parseia o template: substitui {name} e {delta} por {} mantendo o resto.
    // O PRD diz que a interpolação usa o mesmo mecanismo do @log — text_replace_first.
    // Para simplicidade, construímos a mensagem substituindo cada placeholder.
    let mut result_template = String::new();
    let mut has_name = false;
    let mut has_delta = false;
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && i + 1 < chars.len() && chars[i + 1] != '{' {
            // Lê até }
            let mut expr = String::new();
            i += 1;
            while i < chars.len() && chars[i] != '}' {
                expr.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1; // consome }
            }
            let expr = expr.trim();
            match expr {
                "name" => {
                    result_template.push_str("{}");
                    has_name = true;
                }
                "delta" => {
                    result_template.push_str("{}");
                    has_delta = true;
                }
                _ => {
                    // Placeholder desconhecido — mantém literal.
                    result_template.push('{');
                    result_template.push_str(expr);
                    result_template.push('}');
                }
            }
        } else if chars[i] == '{' && i + 1 < chars.len() && chars[i + 1] == '{' {
            result_template.push('{');
            i += 2;
        } else if chars[i] == '}' && i + 1 < chars.len() && chars[i + 1] == '}' {
            result_template.push('}');
            i += 2;
        } else {
            result_template.push(chars[i]);
            i += 1;
        }
    }

    // Aloca o template como string literal (data symbol).
    let template_global = lower.add_string(&result_template);
    let template_ptr = lower
        .builder
        .ins()
        .global_value(lower.module.target_config().pointer_type(), template_global);

    // Primeira substituição: {name} → func_name.
    let msg = if has_name {
        let name_global = lower.add_string(func_name);
        let name_ptr = lower.builder.ins().global_value(
            lower.module.target_config().pointer_type(),
            name_global,
        );
        let replace_fn = lower
            .ffi_refs
            .get("kata_rt_text_replace_first")
            .copied()
            .ok_or_else(|| {
                CodegenError::FfiSymbolNotFound("kata_rt_text_replace_first".into())
            })?;
        let call = lower
            .builder
            .ins()
            .call(replace_fn, &[template_ptr, name_ptr]);
        lower.builder.inst_results(call)[0]
    } else {
        template_ptr
    };

    // Segunda substituição: {delta} → delta em Text.
    let msg = if has_delta {
        let int_to_text_fn = lower
            .ffi_refs
            .get("kata_rt_int_to_text")
            .copied()
            .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_int_to_text".into()))?;
        let call = lower.builder.ins().call(int_to_text_fn, &[delta]);
        let delta_text = lower.builder.inst_results(call)[0];

        let replace_fn = lower
            .ffi_refs
            .get("kata_rt_text_replace_first")
            .copied()
            .ok_or_else(|| {
                CodegenError::FfiSymbolNotFound("kata_rt_text_replace_first".into())
            })?;
        let call = lower.builder.ins().call(replace_fn, &[msg, delta_text]);
        lower.builder.inst_results(call)[0]
    } else {
        msg
    };

    Ok(msg)
}