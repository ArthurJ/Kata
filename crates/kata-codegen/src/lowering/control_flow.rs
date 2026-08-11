//! Lowering de controle de fluxo — arms `Return`, `Loop`, `Break`, `Continue`
//! do match `lower_expr`.
//!
//! Extraído de `expr.rs` para reduzir o tamanho do dispatch central.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{BlockArg, InstBuilder};
use kata_inference::{TypedExpr, TypedExprKind};

use super::LowerCtx;
use super::expr::lower_expr;

/// Lowera arms de controle de fluxo: `Return`, `Loop`, `Break`, `Continue`.
///
/// Retorna `Ok(Some(value))` se o arm foi tratado, `Ok(None)` se o `kind` não é
/// de controle de fluxo (caller continua o match).
pub(crate) fn lower_control_flow(
    expr: &TypedExpr,
    ctx: &mut LowerCtx,
) -> Result<Option<cranelift_codegen::ir::Value>, super::CodegenError> {
    match &expr.kind {
        // ── Return — jump para epilogue_block ──
        TypedExprKind::Return(inner) => {
            let val = lower_expr(&inner.node, ctx)?;
            let epilogue = ctx.epilogue_block.expect("return fora de Action");
            ctx.builder.ins().jump(epilogue, &[BlockArg::Value(val)]);
            // Após jump (terminador), o block está fechado. Não pode adicionar
            // instruções. Retornamos `val` — o caller do loop em define_kata_action
            // detecta Return e break, então este valor é unreachable.
            Ok(Some(val))
        }

        // ── Loop, break, continue ──
        TypedExprKind::Loop { body } => {
            // Cria 3 blocks: loop_block (início do body), continue_block
            // (target de continue), break_block (target de break / saída).
            let loop_block = ctx.builder.create_block();
            let continue_block = ctx.builder.create_block();
            let break_block = ctx.builder.create_block();

            // Salva e configura loop blocks no ctx.
            let prev_break = ctx.loop_break_block;
            let prev_continue = ctx.loop_continue_block;
            ctx.loop_break_block = Some(break_block);
            ctx.loop_continue_block = Some(continue_block);

            // Entra no loop (predecessor 1 de loop_block).
            ctx.builder.ins().jump(loop_block, &[]);

            // Lowera o body no loop_block.
            ctx.builder.switch_to_block(loop_block);
            // Yield point no header do loop. A cada YIELD_INTERVAL
            // iterações, se há outra fiber pronta, suspende cooperativamente.
            // 2 instruções no hot path (dec + branch dentro da FFI).
            let yield_check_ref = ctx
                .ffi_refs
                .get("kata_rt_yield_check")
                .copied()
                .ok_or_else(|| super::CodegenError::FfiSymbolNotFound {
                    symbol: "kata_rt_yield_check".into(),
                })?;
            let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
            ctx.builder.ins().call(yield_check_ref, &[rt_val]);
            let mut hit_terminator = false;
            for e in body {
                lower_expr(&e.node, ctx)?;
                if matches!(
                    e.node.kind,
                    TypedExprKind::Break | TypedExprKind::Continue | TypedExprKind::Return(_)
                ) {
                    hit_terminator = true;
                    break;
                }
            }
            // Fallthrough do body → continue_block (próxima iteração).
            if !hit_terminator {
                ctx.builder.ins().jump(continue_block, &[]);
            }

            // continue_block: jump de volta para loop_block (predecessor 2).
            ctx.builder.switch_to_block(continue_block);
            ctx.builder.ins().jump(loop_block, &[]);

            // Agora que ambos predecessores de loop_block são conhecidos
            // (entry + continue_block), podemos selar.
            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);

            // break_block: retorna Unit.
            ctx.builder.switch_to_block(break_block);
            ctx.builder.seal_block(break_block);
            let unit = ctx.builder.ins().iconst(I64, 0);

            // Restaura ctx.
            ctx.loop_break_block = prev_break;
            ctx.loop_continue_block = prev_continue;

            Ok(Some(unit))
        }

        TypedExprKind::Break => {
            let break_block = ctx
                .loop_break_block
                .expect("break fora de loop (typeck deveria ter rejeitado)");
            // Cria valor Unit ANTES do jump (o jump é terminador e fecha o block).
            let unit = ctx.builder.ins().iconst(I64, 0);
            ctx.builder.ins().jump(break_block, &[]);
            // Após jump (terminador), o block está fechado. O caller detecta
            // Break e não usa o valor de retorno.
            Ok(Some(unit))
        }

        TypedExprKind::Continue => {
            let continue_block = ctx
                .loop_continue_block
                .expect("continue fora de loop (typeck deveria ter rejeitado)");
            let unit = ctx.builder.ins().iconst(I64, 0);
            ctx.builder.ins().jump(continue_block, &[]);
            Ok(Some(unit))
        }

        _ => Ok(None),
    }
}
