//! Helper para iteração de Range — condição de parada e avanço que
//! despacham por tipo (Int = icmp/iadd, Float = float_cmp/fadd).
//!
//! Layout do Range (32 bytes):
//! - offset 0:  start    (i64 — bits de Int SMI ou bits de f64)
//! - offset 8:  step     (i64 — bits de Int SMI ou bits de f64)
//! - offset 16: end      (i64 — bits de Int SMI ou bits de f64)
//! - offset 24: inclusive (i64, SMI: 3 = inclusive, 1 = exclusive)
//!
//! Condição de parada (done = true → break):
//! - step >= 0, exclusive: current >= end
//! - step >= 0, inclusive: current > end
//! - step < 0, exclusive: current <= end
//! - step < 0, inclusive: current < end

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{InstBuilder, MemFlagsData, Value};
use kata_core::ty::Ty;

use super::LowerCtx;

/// Guard de runtime: se step == 0, panic com mensagem graciosa.
///
/// Chamado uma vez antes de iniciar a iteração sobre um Range.
/// O compile-time (check_neutral_step) já rejeita literais zero;
/// este guard pega steps dinâmicos (variáveis) que escapam do typeck.
///
/// Para Int: step SMI == 1 (zero tagged). Para Float: bits == 0.0.
pub(crate) fn range_check_step(coll_val: Value, elem_ty: &Ty, ctx: &mut LowerCtx) {
    let flags = MemFlagsData::new();
    let step_val = ctx.builder.ins().load(I64, flags, coll_val, 8);

    if *elem_ty == Ty::float() {
        // Float: bitcast para F64 e comparar com 0.0
        let cast_flags = MemFlagsData::new();
        let step_f = ctx.builder.ins().bitcast(F64, cast_flags, step_val);
        let zero_f = ctx.builder.ins().f64const(0.0);
        let is_zero = ctx.builder.ins().fcmp(FloatCC::Equal, step_f, zero_f);
        // panic block
        let panic_block = ctx.builder.create_block();
        let ok_block = ctx.builder.create_block();
        ctx.builder.ins().brif(is_zero, panic_block, &[], ok_block, &[]);
        ctx.builder.switch_to_block(panic_block);
        let panic_fn = ctx
            .ffi_refs
            .get("kata_rt_panic")
            .copied()
            .unwrap_or_else(|| panic!("kata_rt_panic not found in ffi_refs"));
        let msg = ctx.builder.ins().iconst(I64, range_step_zero_msg_ptr());
        ctx.builder.ins().call(panic_fn, &[msg]);
        // kata_rt_panic diverges (!) — trap para satisfazer Cranelift.
        ctx.builder.ins().trap(
            cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"),
        );
        ctx.builder.seal_block(panic_block);
        ctx.builder.switch_to_block(ok_block);
        ctx.builder.seal_block(ok_block);
    } else {
        // Int SMI: zero é tag 1 (val 0 << 1 | 1 = 1)
        let zero_smi = ctx.builder.ins().iconst(I64, 1);
        let is_zero = ctx.builder.ins().icmp(IntCC::Equal, step_val, zero_smi);
        let panic_block = ctx.builder.create_block();
        let ok_block = ctx.builder.create_block();
        ctx.builder.ins().brif(is_zero, panic_block, &[], ok_block, &[]);
        ctx.builder.switch_to_block(panic_block);
        let panic_fn = ctx
            .ffi_refs
            .get("kata_rt_panic")
            .copied()
            .unwrap_or_else(|| panic!("kata_rt_panic not found in ffi_refs"));
        let msg = ctx.builder.ins().iconst(I64, range_step_zero_msg_ptr());
        ctx.builder.ins().call(panic_fn, &[msg]);
        // kata_rt_panic diverges (!) — trap para satisfazer Cranelift.
        ctx.builder.ins().trap(
            cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"),
        );
        ctx.builder.seal_block(panic_block);
        ctx.builder.switch_to_block(ok_block);
        ctx.builder.seal_block(ok_block);
    }
}

/// Mensagem C string para panic de step zero.
static RANGE_STEP_ZERO_MSG: &[u8] = b"range step zero - loop infinito evitado (use step != 0)\0";

/// Ponteiro para a mensagem de panic (como i64 para o codegen).
fn range_step_zero_msg_ptr() -> i64 {
    RANGE_STEP_ZERO_MSG.as_ptr() as i64
}

/// Carrega os campos do Range e produz o valor `done` (boolean cranelift).
///
/// Recebe `coll_val` (ponteiro para o struct Range) e `current` (valor
/// atual do iterador). `elem_ty` determina se a comparação é Int (icmp) ou
/// Float (float_cmp). Retorna `done: Value` — quando true, o loop deve
/// parar.
pub(crate) fn range_done(
    coll_val: Value,
    current: Value,
    elem_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Value {
    let flags = MemFlagsData::new();
    let step_val = ctx.builder.ins().load(I64, flags, coll_val, 8);
    let end_val = ctx.builder.ins().load(I64, flags, coll_val, 16);
    let incl_raw = ctx.builder.ins().load(I64, flags, coll_val, 24);
    // SMI: inclusive = 3 (tag 1, value 1), exclusive = 1 (tag 1, value 0)
    // Untag: >> 1 → 1 = inclusive, 0 = exclusive
    let incl_val = ctx.builder.ins().ushr_imm(incl_raw, 1);

    if *elem_ty == Ty::float() {
        range_done_float(current, step_val, end_val, incl_val, ctx)
    } else {
        range_done_int(current, step_val, end_val, incl_val, ctx)
    }
}

/// Condição de parada para Int — usa icmp signed.
fn range_done_int(
    current: Value,
    step_val: Value,
    end_val: Value,
    incl_val: Value,
    ctx: &mut LowerCtx,
) -> Value {
    // Detecta step < 0 (signed)
    let zero_smi = ctx.builder.ins().iconst(I64, 1);
    let step_neg = ctx
        .builder
        .ins()
        .icmp(IntCC::SignedLessThan, step_val, zero_smi);

    // Detecta inclusive (incl_val != 0)
    let is_inclusive = ctx.builder.ins().icmp_imm(IntCC::NotEqual, incl_val, 0);

    // Comparações para step >= 0
    let done_pos_excl = ctx
        .builder
        .ins()
        .icmp(IntCC::SignedGreaterThanOrEqual, current, end_val);
    let done_pos_incl = ctx
        .builder
        .ins()
        .icmp(IntCC::SignedGreaterThan, current, end_val);
    let done_pos = ctx
        .builder
        .ins()
        .select(is_inclusive, done_pos_incl, done_pos_excl);

    // Comparações para step < 0
    let done_neg_excl = ctx
        .builder
        .ins()
        .icmp(IntCC::SignedLessThanOrEqual, current, end_val);
    let done_neg_incl = ctx
        .builder
        .ins()
        .icmp(IntCC::SignedLessThan, current, end_val);
    let done_neg = ctx
        .builder
        .ins()
        .select(is_inclusive, done_neg_incl, done_neg_excl);

    // Seleciona baseado no sinal do step
    ctx.builder.ins().select(step_neg, done_neg, done_pos)
}

/// Condição de parada para Float — usa float_cmp.
///
/// `current`, `step_val`, `end_val` são I64 (bits de f64). Faz bitcast
/// para F64 antes de comparar.
fn range_done_float(
    current: Value,
    step_val: Value,
    end_val: Value,
    incl_val: Value,
    ctx: &mut LowerCtx,
) -> Value {
    let cast_flags = MemFlagsData::new();
    let current_f = ctx.builder.ins().bitcast(F64, cast_flags, current);
    let step_f = ctx.builder.ins().bitcast(F64, cast_flags, step_val);
    let end_f = ctx.builder.ins().bitcast(F64, cast_flags, end_val);

    // Detecta step < 0 (float)
    let zero_f64 = ctx.builder.ins().f64const(0.0);
    let step_neg = ctx.builder.ins().fcmp(FloatCC::LessThan, step_f, zero_f64);

    // Detecta inclusive (incl_val != 0) — incl_val ainda é I64
    let is_inclusive = ctx.builder.ins().icmp_imm(IntCC::NotEqual, incl_val, 0);

    // Comparações para step >= 0
    let done_pos_excl = ctx
        .builder
        .ins()
        .fcmp(FloatCC::GreaterThanOrEqual, current_f, end_f);
    let done_pos_incl = ctx
        .builder
        .ins()
        .fcmp(FloatCC::GreaterThan, current_f, end_f);
    let done_pos = ctx
        .builder
        .ins()
        .select(is_inclusive, done_pos_incl, done_pos_excl);

    // Comparações para step < 0
    let done_neg_excl = ctx
        .builder
        .ins()
        .fcmp(FloatCC::LessThanOrEqual, current_f, end_f);
    let done_neg_incl = ctx.builder.ins().fcmp(FloatCC::LessThan, current_f, end_f);
    let done_neg = ctx
        .builder
        .ins()
        .select(is_inclusive, done_neg_incl, done_neg_excl);

    // Seleciona baseado no sinal do step
    ctx.builder.ins().select(step_neg, done_neg, done_pos)
}

/// Avança o iterador: `current += step`.
///
/// Para Int: `iadd(current, step_val)` + `iadd_imm(-1)` (SMI tag fix).
/// Para Float: bitcast ambos para F64, `fadd`, bitcast de volta para I64.
///
/// Retorna o novo valor de `current`.
pub(crate) fn range_advance(
    coll_val: Value,
    current: Value,
    elem_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Value {
    let flags = MemFlagsData::new();
    let step_val = ctx.builder.ins().load(I64, flags, coll_val, 8);

    if *elem_ty == Ty::float() {
        let cast_flags = MemFlagsData::new();
        let current_f = ctx.builder.ins().bitcast(F64, cast_flags, current);
        let step_f = ctx.builder.ins().bitcast(F64, cast_flags, step_val);
        let next_f = ctx.builder.ins().fadd(current_f, step_f);
        // Bitcast de volta para I64 (preserva os bits de f64)
        ctx.builder.ins().bitcast(I64, cast_flags, next_f)
    } else {
        // SMI: (a<<1|1) + (b<<1|1) = (a+b)<<1 | 2. Subtrair 1 restaura tag.
        let next_raw = ctx.builder.ins().iadd(current, step_val);
        ctx.builder.ins().iadd_imm(next_raw, -1)
    }
}
