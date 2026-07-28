//! Helper para iteração de Range — condição de parada que detecta
//! step negativo e flag inclusive.
//!
//! Layout do Range (32 bytes):
//! - offset 0:  start    (i64)
//! - offset 8:  step     (i64)
//! - offset 16: end      (i64)
//! - offset 24: inclusive (i64, SMI: 3 = inclusive, 1 = exclusive)
//!
//! Condição de parada (done = true → break):
//! - step >= 0, exclusive: current >= end  (SignedGreaterThanOrEqual)
//! - step >= 0, inclusive: current > end   (SignedGreaterThan)
//! - step < 0, exclusive: current <= end  (SignedLessThanOrEqual)
//! - step < 0, inclusive: current < end    (SignedLessThan)

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData, Value};

use super::LowerCtx;

/// Carrega os campos do Range e produz o valor `done` (boolean cranelift).
///
/// Recebe `coll_val` (ponteiro para o struct Range) e `current` (valor
/// atual do iterador). Retorna `done: Value` — quando true, o loop deve
/// parar.
pub(crate) fn range_done(coll_val: Value, current: Value, ctx: &mut LowerCtx) -> Value {
    let flags = MemFlagsData::new();
    let step_val = ctx.builder.ins().load(I64, flags, coll_val, 8);
    let end_val = ctx.builder.ins().load(I64, flags, coll_val, 16);
    let incl_raw = ctx.builder.ins().load(I64, flags, coll_val, 24);
    // SMI: inclusive = 3 (tag 1, value 1), exclusive = 1 (tag 1, value 0)
    // Untag: >> 1 → 1 = inclusive, 0 = exclusive
    let incl_val = ctx.builder.ins().ushr_imm(incl_raw, 1);

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
