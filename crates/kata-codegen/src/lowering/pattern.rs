//! Teste de patterns — usados por clause chain e match.
//!
//! `test_clause_patterns` testa múltiplos patterns contra múltiplos valores
//! (cláusulas multi-arg). `test_single_pattern` testa um pattern contra um
//! valor, com recursão para sub-patterns de tupla.

use cranelift_codegen::ir::types::I64;
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
        TypedPattern::Variant { enum_name, variant } => {
            if enum_name == "Boolean" {
                let expected = if variant == "True" { 1 } else { 0 };
                let eq = lower.builder.ins().icmp_imm(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    val,
                    expected,
                );
                Ok(Some(eq))
            } else {
                Err(super::CodegenError::UnsupportedNode(format!(
                    "Pattern Variant não-Boolean: {enum_name}::{variant}"
                )))
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
