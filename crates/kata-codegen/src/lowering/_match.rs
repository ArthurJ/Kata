//! Lowera um match: branch chain com brif para cada arm.

use cranelift_codegen::ir::InstBuilder;
use kata_inference::{TypedExpr, TypedMatchArm};

use super::expr::lower_expr;
use super::pattern::test_single_pattern;
use super::LowerCtx;
use crate::ffi_sigs::ty_to_clif;

/// Lowera um match: branch chain com brif para cada arm.
pub(crate) fn lower_match(
    scrutinee: &kata_ast::Spanned<TypedExpr>,
    arms: &[TypedMatchArm],
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let scrutinee_val = lower_expr(&scrutinee.node, ctx)?;
    // O tipo de retorno do match é o tipo do body de cada arm.
    // Todos os arms têm o mesmo tipo (verificado pelo typeck).
    let match_ty = &arms[0].body.node.ty;
    let ret_clif = ty_to_clif(match_ty);

    let cont_block = ctx.builder.create_block();
    ctx.builder.append_block_param(cont_block, ret_clif);

    let mut next_test_block = ctx.builder.create_block();
    ctx.builder.ins().jump(next_test_block, &[]);
    ctx.builder.seal_block(next_test_block);

    for arm in arms {
        ctx.builder.switch_to_block(next_test_block);

        let body_block = ctx.builder.create_block();

        let mut pattern_cond = None;

        if let Some(pat) = &arm.pattern {
            pattern_cond = test_single_pattern(pat, scrutinee_val, ctx)?;
        }
        // otherwise (pattern = None): sempre encaixa — pattern_cond = None.

        // Próximo block de teste — sem selar ainda.
        next_test_block = ctx.builder.create_block();

        if let Some(cond) = pattern_cond {
            // Pattern com teste: brif.
            ctx.builder
                .ins()
                .brif(cond, body_block, &[], next_test_block, &[]);
        } else {
            // Pattern incondicional (Ident/Wildcard/otherwise): jump direto.
            ctx.builder.ins().jump(body_block, &[]);
        }
        // Agora que os predecessores foram emitidos, selar.
        ctx.builder.seal_block(next_test_block);

        // Lowera guard (se houver).
        if let Some(guard_expr) = &arm.guard {
            let guard_val = lower_expr(&guard_expr.node, ctx)?;
            let guard_true = ctx.builder.ins().icmp_imm(
                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                guard_val,
                0,
            );
            let guard_fail = ctx.builder.create_block();
            ctx.builder
                .ins()
                .brif(guard_true, body_block, &[], guard_fail, &[]);
            // Agora que o brif adicionou predecessores, selar ambos.
            ctx.builder.seal_block(body_block);
            ctx.builder.seal_block(guard_fail);
            // Em guard_fail: jump para próximo arm.
            ctx.builder.switch_to_block(guard_fail);
            ctx.builder.ins().jump(next_test_block, &[]);
            // Voltar para body_block para lowerar o body.
            ctx.builder.switch_to_block(body_block);
        } else {
            // Sem guard: body_block tem apenas o predecessor do pattern test.
            ctx.builder.seal_block(body_block);
        }

        // Lowera o body do arm.
        ctx.builder.switch_to_block(body_block);
        let body_val = lower_expr(&arm.body.node, ctx)?;
        ctx.builder
            .ins()
            .jump(cont_block, &[cranelift_codegen::ir::BlockArg::Value(body_val)]);
    }

    // Nenhum arm encaixou — runtime trap.
    ctx.builder.switch_to_block(next_test_block);
    ctx.builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"));
    // next_test_block já foi selado dentro do loop.

    ctx.builder.seal_block(cont_block);
    ctx.builder.switch_to_block(cont_block);
    let result = ctx.builder.block_params(cont_block)[0];
    Ok(result)
}