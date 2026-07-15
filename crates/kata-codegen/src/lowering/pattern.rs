//! Teste de patterns — usados por clause chain e match.
//!
//! `test_clause_patterns` testa múltiplos patterns contra múltiplos valores
//! (cláusulas multi-arg). `test_single_pattern` testa um pattern contra um
//! valor, com recursão para sub-patterns de tupla.

use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};
use kata_ast::Spanned;
use kata_inference::TypedPattern;

use super::LowerCtx;
use super::expr::lower_expr;

/// Testa patterns de uma cláusula contra os parâmetros.
/// Retorna `Some(cond_val)` se há um teste condicional (brif), ou `None`
/// se o pattern é incondicional (Ident/Wildcard — sempre encaixa).
pub(crate) fn test_clause_patterns(
    patterns: &[Spanned<TypedPattern>],
    params: &[cranelift_codegen::ir::Value],
    lower: &mut LowerCtx,
) -> Result<Option<cranelift_codegen::ir::Value>, super::CodegenError> {
    let mut all_matches = None;

    for (pat, val) in patterns.iter().zip(params.iter()) {
        if let Some(cond) = test_single_pattern(pat, *val, lower)? {
            all_matches = Some(match all_matches {
                None => cond,
                Some(prev) => lower.builder.ins().band(prev, cond),
            });
        }
    }

    Ok(all_matches)
}

/// Testa um único pattern contra um valor. Retorna `Some(cond)` se há teste
/// condicional, `None` se o pattern é incondicional (Ident/Wildcard).
/// Usado por `TypedPattern::Tuple` para recursão sobre sub-patterns.
pub(crate) fn test_single_pattern(
    pat: &Spanned<TypedPattern>,
    val: cranelift_codegen::ir::Value,
    lower: &mut LowerCtx,
) -> Result<Option<cranelift_codegen::ir::Value>, super::CodegenError> {
    match &pat.node {
        TypedPattern::Ident { name, ty } => {
            let clif_ty = crate::ffi_sigs::ty_to_clif(ty);
            // Se o valor recebido é I64 mas a var é F64 (ex: payload de Sum
            // extraído como I64, mas o binding é Float), bitcast I64→F64.
            let val = {
                let val_ty = lower.builder.func.dfg.value_type(val);
                if val_ty == I64 && clif_ty == cranelift_codegen::ir::types::F64 {
                    lower.builder.ins().bitcast(F64, MemFlagsData::new(), val)
                } else if val_ty == F64 && clif_ty == I64 {
                    lower.builder.ins().bitcast(I64, MemFlagsData::new(), val)
                } else {
                    val
                }
            };
            let var = lower.new_var(name, clif_ty);
            lower.builder.def_var(var, val);
            Ok(None)
        }
        TypedPattern::Wildcard => Ok(None),
        TypedPattern::Literal { value } => {
            let lit_val = lower_expr(&value.node, lower)?;
            let eq = lower.builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::Equal,
                val,
                lit_val,
            );
            Ok(Some(eq))
        }
        TypedPattern::Variant {
            enum_name,
            variant,
            sub_patterns,
            tag,
        } => {
            if enum_name == "Boolean" {
                let expected = if variant == "True" { 1 } else { 0 };
                let eq = lower.builder.ins().icmp_imm(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    val,
                    expected,
                );
                Ok(Some(eq))
            } else {
                // Fase 5: Sum com payload — extrair tag e comparar.
                let tag_func = lower.ffi_refs.get("kata_rt_sum_tag_int").ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_sum_tag_int".into())
                })?;
                let tag_call = lower.builder.ins().call(*tag_func, &[val]);
                let actual_tag = lower.builder.inst_results(tag_call)[0];

                // Compara tag extraída com tag esperada (índice da variante).
                let expected_tag = lower.builder.ins().iconst(I64, *tag as i64);
                let eq = lower.builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    actual_tag,
                    expected_tag,
                );

                // Se há sub-patterns, extrair payload e testar sub-pattern.
                if let Some(subs) = sub_patterns {
                    // Payload está no offset 8 do box.
                    let flags = MemFlagsData::new();
                    let payload_val = lower.builder.ins().load(I64, flags, val, 8);
                    // Testa o sub-pattern contra o payload extraído.
                    // Por enquanto, 1 sub-pattern (Fase 5 não suporta multi-payload).
                    let sub_cond = test_single_pattern(&subs[0], payload_val, lower)?;
                    // Combina: tag == expected AND sub_pattern match.
                    if let Some(cond) = sub_cond {
                        let combined = lower.builder.ins().band(eq, cond);
                        Ok(Some(combined))
                    } else {
                        // Sub-pattern é incondicional (Ident/Wildcard) — só precisa tag match.
                        // Mas precisamos fazer o binding do sub-pattern (def_var).
                        // O test_single_pattern já faz def_var para Ident.
                        Ok(Some(eq))
                    }
                } else {
                    Ok(Some(eq))
                }
            }
        }
        TypedPattern::Tuple { elements } => {
            let flags = MemFlagsData::new();
            let mut all_matches = None;
            for (i, sub_pat) in elements.iter().enumerate() {
                let offset = (i * 8) as i32;
                let elem_val = lower.builder.ins().load(I64, flags, val, offset);
                let sub_cond = test_single_pattern(sub_pat, elem_val, lower)?;
                if let Some(cond) = sub_cond {
                    all_matches = Some(match all_matches {
                        None => cond,
                        Some(prev) => lower.builder.ins().band(prev, cond),
                    });
                }
            }
            Ok(all_matches)
        }
        TypedPattern::Cons { .. } => Err(super::CodegenError::UnsupportedNode(
            "Pattern Cons: List é Fio 8".into(),
        )),
    }
}
