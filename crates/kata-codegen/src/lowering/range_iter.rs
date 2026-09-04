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

/// Computa `len` de um Range O(1) — aritmética sobre start/step/end/inclusive.
///
/// Recebe `coll_val` (ponteiro para o struct Range) e `elem_ty` (Int ou Float).
/// Retorna SMI-tagged Int com a contagem de elementos.
///
/// Fórmula (após untag/decode):
/// - diff = step > 0 ? (end - start) : (start - end)
/// - abs_step = |step|
/// - exclusive: count = floor(diff / abs_step)
/// - inclusive: count = floor(diff / abs_step) + 1
/// - clamp para 0 se count < 0 (range degenerado/vazio)
///
/// Para Int: aritmética SMI untagged via sshr_imm(1), sdiv, depois retag com
/// ishl(1) + bor(1).
///
/// Para Float: bitcast para F64, fdiv, floor via `kata_rt_floor` FFI (que
/// retorna SMI-tagged), depois soma 1 (iadd_imm(-1) para manter SMI tag)
/// se inclusive.
pub(crate) fn range_len(
    coll_val: Value,
    elem_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Value {
    let flags = MemFlagsData::new();
    let start = ctx.builder.ins().load(I64, flags, coll_val, 0);
    let step = ctx.builder.ins().load(I64, flags, coll_val, 8);
    let end = ctx.builder.ins().load(I64, flags, coll_val, 16);
    let incl_raw = ctx.builder.ins().load(I64, flags, coll_val, 24);
    // SMI: inclusive = 3 (tag 1, value 1), exclusive = 1 (tag 1, value 0)
    // Untag: >> 1 → 1 = inclusive, 0 = exclusive
    let incl_val = ctx.builder.ins().ushr_imm(incl_raw, 1);

    if *elem_ty == Ty::float() {
        range_len_float(start, step, end, incl_val, ctx)
    } else {
        range_len_int(start, step, end, incl_val, ctx)
    }
}

/// `len` para Range de Int — aritmética SMI inline.
fn range_len_int(
    start: Value,
    step: Value,
    end: Value,
    incl_val: Value,
    ctx: &mut LowerCtx,
) -> Value {
    // Decodificar SMI (>>1, signed shift) para aritmética com sinal.
    let start_dec = ctx.builder.ins().sshr_imm(start, 1);
    let step_dec = ctx.builder.ins().sshr_imm(step, 1);
    let end_dec = ctx.builder.ins().sshr_imm(end, 1);

    // diff = step > 0 ? (end - start) : (start - end)
    let zero = ctx.builder.ins().iconst(I64, 0);
    let step_pos = ctx.builder.ins().icmp(IntCC::SignedGreaterThan, step_dec, zero);
    let diff_pos = ctx.builder.ins().isub(end_dec, start_dec);
    let diff_neg = ctx.builder.ins().isub(start_dec, end_dec);
    let diff = ctx.builder.ins().select(step_pos, diff_pos, diff_neg);

    // abs_step = step > 0 ? step : -step
    let neg_step = ctx.builder.ins().ineg(step_dec);
    let abs_step = ctx.builder.ins().select(step_pos, step_dec, neg_step);

    // count = ceil(diff / abs_step) (exclusive) ou floor(diff / abs_step)+1 (inclusive)
    // sdiv trunca para zero = floor para operandos ≥ 0.
    // ceil(a/b) = (a + b - 1) / b.
    // Guard: se abs_step == 0, retornar 0.
    let is_zero_step = ctx.builder.ins().icmp(IntCC::Equal, abs_step, zero);
    let floor_div = ctx.builder.ins().sdiv(diff, abs_step);
    let one = ctx.builder.ins().iconst(I64, 1);
    let ceil_num = ctx.builder.ins().iadd(diff, abs_step);
    let ceil_num = ctx.builder.ins().isub(ceil_num, one);
    let ceil_div = ctx.builder.ins().sdiv(ceil_num, abs_step);

    let is_inclusive = ctx.builder.ins().icmp_imm(IntCC::NotEqual, incl_val, 0);
    let incl_count = ctx.builder.ins().iadd(floor_div, one);
    let count = ctx.builder.ins().select(is_inclusive, incl_count, ceil_div);

    // Se step == 0: return 0
    let result = ctx.builder.ins().select(is_zero_step, zero, count);

    // clamp negativo para 0
    let is_neg = ctx.builder.ins().icmp(IntCC::SignedLessThan, result, zero);
    let clamped = ctx.builder.ins().select(is_neg, zero, result);

    // Retag como SMI: (val << 1) | 1
    let shifted = ctx.builder.ins().ishl(clamped, one);
    ctx.builder.ins().bor_imm(shifted, 1)
}

/// `len` para Range de Float — aritmética via F64 + kata_rt_ceil/floor.
fn range_len_float(
    start: Value,
    step: Value,
    end: Value,
    incl_val: Value,
    ctx: &mut LowerCtx,
) -> Value {
    let cast_flags = MemFlagsData::new();
    let start_f = ctx.builder.ins().bitcast(F64, cast_flags, start);
    let step_f = ctx.builder.ins().bitcast(F64, cast_flags, step);
    let end_f = ctx.builder.ins().bitcast(F64, cast_flags, end);

    // diff = step > 0 ? (end - start) : (start - end)
    let zero_f = ctx.builder.ins().f64const(0.0);
    let step_pos = ctx.builder.ins().fcmp(FloatCC::GreaterThan, step_f, zero_f);
    let diff_pos = ctx.builder.ins().fsub(end_f, start_f);
    let diff_neg = ctx.builder.ins().fsub(start_f, end_f);
    let diff = ctx.builder.ins().select(step_pos, diff_pos, diff_neg);

    // abs_step = step > 0 ? step : -step
    let neg_step = ctx.builder.ins().fneg(step_f);
    let abs_step = ctx.builder.ins().select(step_pos, step_f, neg_step);

    // exclusive: count = ceil(ratio) = floor(ratio) + (diff > floor_div * abs_step ? 1 : 0)
    // inclusive: count = floor(ratio) + 1
    // Usa apenas kata_rt_floor (SMI-tagged). Verifica remainder aritmeticamente
    // para evitar conversão SMI→Float.
    let ratio = ctx.builder.ins().fdiv(diff, abs_step);
    let floor_ref = ctx
        .ffi_refs
        .get("kata_rt_floor")
        .copied()
        .unwrap_or_else(|| panic!("kata_rt_floor not found in ffi_refs"));
    let floor_call = ctx.builder.ins().call(floor_ref, &[ratio]);
    let floor_smi = ctx.builder.inst_results(floor_call)[0]; // SMI-tagged Int

    // Converter floor_smi (SMI-tagged I64) → F64 via kata_rt_int_to_float FFI.
    // SMI untag não é suficiente — precisamos do valor Float para comparar com ratio.
    let i2f_ref = ctx
        .ffi_refs
        .get("kata_rt_int_to_float")
        .copied()
        .unwrap_or_else(|| panic!("kata_rt_int_to_float not found in ffi_refs"));
    let i2f_call = ctx.builder.ins().call(i2f_ref, &[floor_smi]);
    let floor_f = ctx.builder.inst_results(i2f_call)[0]; // F64
    let has_remainder = ctx.builder.ins().fcmp(FloatCC::GreaterThan, ratio, floor_f);

    // exclusive: ceil = floor + (has_remainder ? 1 : 0)
    let one_smi = ctx.builder.ins().iconst(I64, 3); // SMI 1 = (1<<1)|1 = 3
    let zero_smi_raw = ctx.builder.ins().iconst(I64, 1); // SMI 0
    let ceil_add = ctx.builder.ins().select(has_remainder, one_smi, zero_smi_raw);
    let ceil_smi = ctx.builder.ins().iadd(floor_smi, ceil_add);
    // Corrigir tag: iadd de dois SMIs = (a+b)<<1|2. Subtrair 1 para restaurar tag.
    let ceil_smi = ctx.builder.ins().iadd_imm(ceil_smi, -1);

    // inclusive: floor + 1 (SMI: iadd_imm(-1) inverte o tag — +1 real = -1 raw)
    // Usar uma cópia independente do floor_smi para evitar reuso de SSA.
    // NOTA: floor_smi já é usado no path exclusive acima. No Cranelift SSA,
    // reuso de valor deveria ser seguro, mas há um crash neste exato path.
    // Como workaround, não reusar floor_smi — re-calcular incl_count via
    // select entre ceil_smi e floor_smi+1.
    // Na verdade: inclusive = ceil_smi + (has_remainder ? 0 : 1) = floor + 1 sempre.
    // Simplificando: inclusive = ceil_smi se has_remainder, senão ceil_smi + 1_smi.
    // Mas isso é equivalente a floor_smi + 1 sempre.
    // Vamos tentar: incl_count = iadd(floor_smi, one_smi) - 1 (tag fix).
    let incl_count = ctx.builder.ins().iadd(floor_smi, one_smi);
    let incl_count = ctx.builder.ins().iadd_imm(incl_count, -1);
    let is_inclusive = ctx.builder.ins().icmp_imm(IntCC::NotEqual, incl_val, 0);
    let count_final = ctx.builder.ins().select(is_inclusive, incl_count, ceil_smi);

    // Guard: se abs_step == 0.0 (step zero), return SMI 0
    let is_zero_step = ctx.builder.ins().fcmp(FloatCC::Equal, abs_step, zero_f);
    let smi_zero = ctx.builder.ins().iconst(I64, 1); // SMI 0
    let result = ctx.builder.ins().select(is_zero_step, smi_zero, count_final);

    // clamp: se count < 0 (SMI signed), return SMI 0
    let zero_smi = ctx.builder.ins().iconst(I64, 1);
    let is_neg = ctx.builder.ins().icmp(IntCC::SignedLessThan, result, zero_smi);
    ctx.builder.ins().select(is_neg, smi_zero, result)
}

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
        ctx.builder
            .ins()
            .brif(is_zero, panic_block, &[], ok_block, &[]);
        ctx.builder.switch_to_block(panic_block);
        let panic_fn = ctx
            .ffi_refs
            .get("kata_rt_panic")
            .copied()
            .unwrap_or_else(|| panic!("kata_rt_panic not found in ffi_refs"));
        let msg = ctx.builder.ins().iconst(I64, range_step_zero_msg_ptr());
        ctx.builder.ins().call(panic_fn, &[msg]);
        // kata_rt_panic diverges (!) — trap para satisfazer Cranelift.
        ctx.builder
            .ins()
            .trap(cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"));
        ctx.builder.seal_block(panic_block);
        ctx.builder.switch_to_block(ok_block);
        ctx.builder.seal_block(ok_block);
    } else {
        // Int SMI: zero é tag 1 (val 0 << 1 | 1 = 1)
        let zero_smi = ctx.builder.ins().iconst(I64, 1);
        let is_zero = ctx.builder.ins().icmp(IntCC::Equal, step_val, zero_smi);
        let panic_block = ctx.builder.create_block();
        let ok_block = ctx.builder.create_block();
        ctx.builder
            .ins()
            .brif(is_zero, panic_block, &[], ok_block, &[]);
        ctx.builder.switch_to_block(panic_block);
        let panic_fn = ctx
            .ffi_refs
            .get("kata_rt_panic")
            .copied()
            .unwrap_or_else(|| panic!("kata_rt_panic not found in ffi_refs"));
        let msg = ctx.builder.ins().iconst(I64, range_step_zero_msg_ptr());
        ctx.builder.ins().call(panic_fn, &[msg]);
        // kata_rt_panic diverges (!) — trap para satisfazer Cranelift.
        ctx.builder
            .ins()
            .trap(cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"));
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
