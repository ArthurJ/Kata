//! Lowering de `ForIn` — loop inlined por tipo concreto (List/Array/Range).
//!
//! Extraído de `collections_literal.rs`.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};

use kata_ast::Spanned;
use kata_core::ty::Ty;
use kata_inference::TypedExpr;

use super::LowerCtx;
use super::expr::lower_expr;


/// Lowera `ForIn { var_name, var_ty, iterable, body }`:
///
/// Loop inlined por tipo concreto do iterable:
/// - `Ty::List` — percorre Cons cells
/// - `Ty::Array` — percorre índices 0..len
/// - `Ty::Range` — percorre start..end com step
pub(crate) fn lower_for_in(
    var_name: &str,
    var_ty: &Ty,
    iterable: &Spanned<TypedExpr>,
    body: &[Spanned<TypedExpr>],
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // Salva loop blocks anteriores.
    let prev_break = ctx.loop_break_block;
    let prev_continue = ctx.loop_continue_block;

    let loop_block = ctx.builder.create_block();
    let continue_block = ctx.builder.create_block();
    let break_block = ctx.builder.create_block();
    ctx.loop_break_block = Some(break_block);
    ctx.loop_continue_block = Some(continue_block);

    let coll_val = lower_expr(&iterable.node, ctx)?;
    let coll_ty = &iterable.node.ty;

    match coll_ty {
        Ty::List(_) => {
            // List: percorre Cons cells. current = coll_ptr.
            // Condição: current != 0 (Nil).
            let current_var = ctx.new_var("__for_current", I64);
            ctx.builder.def_var(current_var, coll_val);

            ctx.builder.ins().jump(loop_block, &[]);
            ctx.builder.switch_to_block(loop_block);
            // Yield point no header.
            let yc = ctx
                .ffi_refs
                .get("kata_rt_yield_check")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_yield_check".into())
                })?;
            ctx.builder.ins().call(yc, &[]);
            let current = ctx.builder.use_var(current_var);
            let is_nil = ctx.builder.ins().icmp_imm(
                cranelift_codegen::ir::condcodes::IntCC::Equal,
                current,
                0,
            );
            ctx.builder
                .ins()
                .brif(is_nil, break_block, &[], continue_block, &[]);

            ctx.builder.switch_to_block(continue_block);
            // head = load current+0, tail = load current+8
            let flags = MemFlagsData::new();
            let head_val = ctx.builder.ins().load(I64, flags, current, 0);
            // Bitcast I64→F64 se var_ty é Float.
            let head_val = if *var_ty == Ty::float() {
                ctx.builder.ins().bitcast(
                    cranelift_codegen::ir::types::F64,
                    MemFlagsData::new(),
                    head_val,
                )
            } else {
                head_val
            };
            let tail_val = ctx.builder.ins().load(I64, flags, current, 8);

            let elem_var = ctx.new_var(var_name, super::resolve_clif_ty(var_ty, ctx.struct_registry));
            ctx.builder.def_var(elem_var, head_val);
            ctx.builder.def_var(current_var, tail_val);

            // Executa body.
            for e in body {
                lower_expr(&e.node, ctx)?;
            }
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Array(_) => {
            // Array: percorre índices 0..len.
            // len = load coll_ptr+0
            let flags = MemFlagsData::new();
            let len_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let idx_var = ctx.new_var("__for_idx", I64);
            let zero = ctx.builder.ins().iconst(I64, 0);
            ctx.builder.def_var(idx_var, zero);

            ctx.builder.ins().jump(loop_block, &[]);
            ctx.builder.switch_to_block(loop_block);
            // Yield point no header.
            let yc = ctx
                .ffi_refs
                .get("kata_rt_yield_check")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_yield_check".into())
                })?;
            ctx.builder.ins().call(yc, &[]);
            let idx = ctx.builder.use_var(idx_var);
            let done = ctx.builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                idx,
                len_val,
            );
            ctx.builder
                .ins()
                .brif(done, break_block, &[], continue_block, &[]);

            ctx.builder.switch_to_block(continue_block);
            // elem = load coll_ptr + 8 + idx * 8
            let offset = ctx.builder.ins().imul_imm(idx, 8);
            let data_ptr = ctx.builder.ins().iadd_imm(coll_val, 8);
            let elem_ptr = ctx.builder.ins().iadd(data_ptr, offset);
            let elem_val = ctx.builder.ins().load(I64, flags, elem_ptr, 0);
            let elem_val = if *var_ty == Ty::float() {
                ctx.builder.ins().bitcast(
                    cranelift_codegen::ir::types::F64,
                    MemFlagsData::new(),
                    elem_val,
                )
            } else {
                elem_val
            };
            let elem_var = ctx.new_var(var_name, super::resolve_clif_ty(var_ty, ctx.struct_registry));
            ctx.builder.def_var(elem_var, elem_val);

            // idx += 1
            let next_idx = ctx.builder.ins().iadd_imm(idx, 1);
            ctx.builder.def_var(idx_var, next_idx);

            for e in body {
                lower_expr(&e.node, ctx)?;
            }
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Range(_) => {
            // Range: percorre current = start, current += step,
            // condição: inclusive ? current > end : current >= end.
            let flags = MemFlagsData::new();
            let start_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let step_val = ctx.builder.ins().load(I64, flags, coll_val, 8);
            let end_val = ctx.builder.ins().load(I64, flags, coll_val, 16);
            let current_var = ctx.new_var("__for_current", I64);
            ctx.builder.def_var(current_var, start_val);

            ctx.builder.ins().jump(loop_block, &[]);
            ctx.builder.switch_to_block(loop_block);
            // Yield point no header.
            let yc = ctx
                .ffi_refs
                .get("kata_rt_yield_check")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_yield_check".into())
                })?;
            ctx.builder.ins().call(yc, &[]);
            let current = ctx.builder.use_var(current_var);
            // Para inclusive: current > end → break
            // Para exclusive: current >= end → break
            // (detectado pelo campo `inclusive` na TAST, mas não temos
            // acesso aqui — usar inclusive do match pattern)
            let done = ctx.builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                current,
                end_val,
            );
            ctx.builder
                .ins()
                .brif(done, break_block, &[], continue_block, &[]);

            ctx.builder.switch_to_block(continue_block);
            let elem_var = ctx.new_var(var_name, super::resolve_clif_ty(var_ty, ctx.struct_registry));
            let elem_val = if *var_ty == Ty::float() {
                ctx.builder.ins().bitcast(
                    cranelift_codegen::ir::types::F64,
                    MemFlagsData::new(),
                    current,
                )
            } else {
                current
            };
            ctx.builder.def_var(elem_var, elem_val);

            // current += step
            let next_raw = ctx.builder.ins().iadd(current, step_val);
            // SMI fix: (a<<1|1) + (b<<1|1) = (a+b)<<1 | 2. Subtrair 1 restaura tag.
            let next = ctx.builder.ins().iadd_imm(next_raw, -1);
            ctx.builder.def_var(current_var, next);

            for e in body {
                lower_expr(&e.node, ctx)?;
            }
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        _ => {
            return Err(super::CodegenError::UnsupportedNode(format!(
                "ForIn sobre tipo não-iterável: {coll_ty:?}"
            )));
        }
    }

    // break_block: retorna Unit.
    ctx.builder.switch_to_block(break_block);
    ctx.builder.seal_block(break_block);
    let unit = ctx.builder.ins().iconst(I64, 0);

    // Restaura ctx.
    ctx.loop_break_block = prev_break;
    ctx.loop_continue_block = prev_continue;

    Ok(unit)
}
