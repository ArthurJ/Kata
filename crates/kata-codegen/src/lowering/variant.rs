//! Lowering de variants de enum (Sum types) — extrato de `expr.rs`.
//!
//! `VariantQual` (variante unitária, ex: `Boolean::True`) e
//! `VariantConstruct` (variante com payload, ex: `Result::Ok 42`) compartilham
//! a infraestrutura de `kata_rt_store_sum_result(tag, payload, arena)` e a
//! seleção de arena baseada em `EscapeTarget`.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};
use kata_inference::TypedExpr;

use super::LowerCtx;
use super::expr::lower_expr;

/// Lowera `Enum::Variante` (unitária). Boolean é inline (1/0); demais enums
/// chamam `kata_rt_store_sum_result(tag, 0, arena)`.
pub(crate) fn lower_variant_qual(
    expr: &TypedExpr,
    enum_name: &str,
    variant: &str,
    tag: &usize,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    if enum_name == "Boolean" {
        let val = if variant == "True" { 1 } else { 0 };
        return Ok(ctx.builder.ins().iconst(I64, val));
    }
    // Variante unitária de enum do usuário: box com tag, payload = 0.
    let tag_val = ctx.builder.ins().iconst(I64, *tag as i64);
    let payload_val = ctx.builder.ins().iconst(I64, 0);
    let arena_handle = arena_handle_for(expr, ctx);
    let func_ref = ctx
        .ffi_refs
        .get("kata_rt_store_sum_result")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_store_sum_result".into()))?;
    let call_inst = ctx
        .builder
        .ins()
        .call(*func_ref, &[tag_val, payload_val, arena_handle]);
    Ok(ctx.builder.inst_results(call_inst)[0])
}

/// Lowera `Enum::Variante payload` (com payload). Faz bitcast F64→I64 quando
/// necessário e chama `kata_rt_store_sum_result(tag, payload, arena)`.
pub(crate) fn lower_variant_construct(
    expr: &TypedExpr,
    payload: &kata_ast::Spanned<TypedExpr>,
    tag: &usize,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let payload_val = lower_expr(&payload.node, ctx)?;

    // Bitcast F64→I64 se necessário: store_sum_result espera I64 para o
    // payload, mas Float lowera como F64.
    let payload_val = {
        let payload_ty = ctx.builder.func.dfg.value_type(payload_val);
        if payload_ty == cranelift_codegen::ir::types::F64 {
            ctx.builder
                .ins()
                .bitcast(I64, MemFlagsData::new(), payload_val)
        } else {
            payload_val
        }
    };

    let tag_val = ctx.builder.ins().iconst(I64, *tag as i64);
    let arena_handle = arena_handle_for(expr, ctx);

    let func_ref = ctx
        .ffi_refs
        .get("kata_rt_store_sum_result")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_store_sum_result".into()))?;
    let call_inst = ctx
        .builder
        .ins()
        .call(*func_ref, &[tag_val, payload_val, arena_handle]);
    Ok(ctx.builder.inst_results(call_inst)[0])
}

/// Seleciona o handle de arena conforme `EscapeTarget`:
/// `Local` → fiber_arena, `Caller` → caller_arena, `Heap` → root_arena.
/// Fallback para `iconst 0` quando a arena não está disponível (entry point).
fn arena_handle_for(expr: &TypedExpr, ctx: &mut LowerCtx) -> cranelift_codegen::ir::Value {
    crate::lowering::escape_arena::arena_handle_for_escape(expr.escape, ctx)
}
