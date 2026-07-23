//! Helpers de lowering para `DictLit` e `SetLit` — hash/eq FFI resolution,
//! `bitcast_to_i64`, `get_ffi_fn_ptr`, e as funções `lower_dict_lit` /
//! `lower_set_lit`.
//!
//! Extraído de `collections_literal.rs`.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{GlobalValueData, InstBuilder, MemFlagsData};

use kata_ast::Spanned;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::TypedExpr;

use super::LowerCtx;
use super::expr::lower_expr;

/// Resolve o hash FFI function name para um dado key type.
pub(crate) fn hash_fn_name(key_ty: &Ty) -> Result<&'static str, super::CodegenError> {
    match key_ty {
        Ty::Prim(PrimTy::Int) => Ok("kata_rt_hash_int"),
        Ty::Prim(PrimTy::Text) => Ok("kata_rt_hash_text"),
        Ty::Prim(PrimTy::Rational) => Ok("kata_rt_hash_rational"),
        _ => Err(super::CodegenError::UnsupportedNode(format!(
            "DictLit/SetLit: tipo de chave não-hashable: {key_ty}"
        ))),
    }
}

/// Resolve o eq FFI function name para um dado key type.
pub(crate) fn eq_fn_name(key_ty: &Ty) -> Result<&'static str, super::CodegenError> {
    match key_ty {
        Ty::Prim(PrimTy::Int) => Ok("kata_rt_bi_eq"),
        Ty::Prim(PrimTy::Text) => Ok("kata_rt_string_eq"),
        _ => Err(super::CodegenError::UnsupportedNode(format!(
            "DictLit/SetLit: tipo de chave sem eq_fn: {key_ty}"
        ))),
    }
}

/// Obtém o ponteiro de uma função FFI como valor i64 (via GlobalValue::Symbol).
///
/// Segue o mesmo padrão usado em `action_call.rs` para obter function pointers:
/// 1. `declare_func_in_func` → FuncRef
/// 2. `ext_funcs[func_ref].name` → ExternalName
/// 3. `create_global_value(GlobalValueData::Symbol { name, ... })` → GlobalValue
/// 4. `global_value(pointer_type, gv)` → Value (fn_ptr)
pub(crate) fn get_ffi_fn_ptr(
    ffi_name: &str,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let fid = ctx
        .ffi_ids
        .get(ffi_name)
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(ffi_name.to_string()))?;
    let func_ref = ctx.module.declare_func_in_func(*fid, ctx.builder.func);
    let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
    let func_gv = ctx
        .builder
        .func
        .create_global_value(GlobalValueData::Symbol {
            name: ext_func_name,
            offset: 0.into(),
            // FFI imports are in the host binary, not colocated with JIT code.
            // colocated: true → PC-relative (PCRel4) → i32 overflow if too far.
            // colocated: false → absolute (Abs8) → works regardless of distance.
            colocated: false,
            tls: false,
        });
    Ok(ctx
        .builder
        .ins()
        .global_value(ctx.module.target_config().pointer_type(), func_gv))
}

/// Bitcast F64→I64 se necessário (mesmo pattern do ArrayLit/ListLit).
pub(crate) fn bitcast_to_i64(
    val: cranelift_codegen::ir::Value,
    ctx: &mut LowerCtx,
) -> cranelift_codegen::ir::Value {
    let ty = ctx.builder.func.dfg.value_type(val);
    if ty == cranelift_codegen::ir::types::F64 {
        ctx.builder.ins().bitcast(I64, MemFlagsData::new(), val)
    } else {
        val
    }
}

/// Lowera `DictLit { entries, key_ty, value_ty }`:
///
/// 1. dict = kata_rt_dict_empty(arena)
/// 2. Para cada (key, val):
///    a. hash = hash_fn(key)
///    b. eq_fn_ptr = get_ffi_fn_ptr(eq_fn_name)
///    c. dict = kata_rt_dict_insert(dict, key, val, hash, eq_fn_ptr, arena)
/// 3. Retorna dict
pub(crate) fn lower_dict_lit(
    entries: &[(Spanned<TypedExpr>, Spanned<TypedExpr>)],
    key_ty: &Ty,
    _value_ty: &Ty,
    _expr: &TypedExpr,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let arena_handle = ctx
        .fiber_arena
        .or(ctx.caller_arena)
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // dict = kata_rt_dict_empty(arena)
    let empty_ref = ctx
        .ffi_refs
        .get("kata_rt_dict_empty")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_dict_empty".into()))?;
    let call = ctx.builder.ins().call(*empty_ref, &[arena_handle]);
    let mut dict = ctx.builder.inst_results(call)[0];

    // Resolve hash e eq function names.
    let hash_name = hash_fn_name(key_ty)?;
    let eq_name = eq_fn_name(key_ty)?;
    let hash_ref = ctx
        .ffi_refs
        .get(hash_name)
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(hash_name.into()))?;
    let insert_ref = ctx
        .ffi_refs
        .get("kata_rt_dict_insert")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_dict_insert".into()))?;

    // Resolve eq_fn pointer ONCE (outside loop) — creating multiple GlobalValues
    // for the same symbol in a single function body can corrupt the pointer.
    let eq_fn_ptr = get_ffi_fn_ptr(eq_name, ctx)?;

    for (key_expr, val_expr) in entries {
        let key_val = lower_expr(&key_expr.node, ctx)?;
        let key_val = bitcast_to_i64(key_val, ctx);
        let val_val = lower_expr(&val_expr.node, ctx)?;
        let val_val = bitcast_to_i64(val_val, ctx);

        // hash = hash_fn(key)
        let hash_call = ctx.builder.ins().call(hash_ref, &[key_val]);
        let hash_val = ctx.builder.inst_results(hash_call)[0];

        // dict = kata_rt_dict_insert(dict, key, val, hash, eq_fn_ptr, arena)
        let insert_call = ctx.builder.ins().call(
            insert_ref,
            &[dict, key_val, val_val, hash_val, eq_fn_ptr, arena_handle],
        );
        dict = ctx.builder.inst_results(insert_call)[0];
    }

    Ok(dict)
}

/// Lowera `SetLit { elements, elem_ty }`:
///
/// 1. set = kata_rt_set_empty(arena)
/// 2. Para cada elem:
///    a. hash = hash_fn(elem)
///    b. eq_fn_ptr = get_ffi_fn_ptr(eq_fn_name)
///    c. set = kata_rt_set_insert(set, elem, hash, eq_fn_ptr, arena)
/// 3. Retorna set
pub(crate) fn lower_set_lit(
    elements: &[Spanned<TypedExpr>],
    elem_ty: &Ty,
    _expr: &TypedExpr,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let arena_handle = ctx
        .fiber_arena
        .or(ctx.caller_arena)
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // set = kata_rt_set_empty(arena)
    let empty_ref = ctx
        .ffi_refs
        .get("kata_rt_set_empty")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_set_empty".into()))?;
    let call = ctx.builder.ins().call(*empty_ref, &[arena_handle]);
    let mut set = ctx.builder.inst_results(call)[0];

    // Resolve hash e eq function names.
    let hash_name = hash_fn_name(elem_ty)?;
    let eq_name = eq_fn_name(elem_ty)?;
    let hash_ref = ctx
        .ffi_refs
        .get(hash_name)
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(hash_name.into()))?;
    let insert_ref = ctx
        .ffi_refs
        .get("kata_rt_set_insert")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_set_insert".into()))?;

    // Resolve eq_fn pointer ONCE (outside loop).
    let eq_fn_ptr = get_ffi_fn_ptr(eq_name, ctx)?;

    for elem in elements {
        let elem_val = lower_expr(&elem.node, ctx)?;
        let elem_val = bitcast_to_i64(elem_val, ctx);

        // hash = hash_fn(elem)
        let hash_call = ctx.builder.ins().call(hash_ref, &[elem_val]);
        let hash_val = ctx.builder.inst_results(hash_call)[0];

        // set = kata_rt_set_insert(set, elem, hash, eq_fn_ptr, arena)
        let insert_call = ctx.builder.ins().call(
            insert_ref,
            &[set, elem_val, hash_val, eq_fn_ptr, arena_handle],
        );
        set = ctx.builder.inst_results(insert_call)[0];
    }

    Ok(set)
}
