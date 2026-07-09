//! Lowering de cláusulas lambda: guards, branch chain, with bindings.
//!
//! Funções neste módulo:
//! - `lower_clause_body` — despacha para guards ou body direto
//! - `lower_guards` — branch chain de guards dentro de uma cláusula
//! - `lower_clause_chain` — múltiplas cláusulas como branch chain (pattern matching)
//! - `lower_with_bindings` — bindings `with` (computações prévias)
//! - `all_patterns_are_ident` — predicate para fast-path
//! - `bind_patterns_to_params` — binda patterns Ident aos block params

use cranelift_codegen::ir::{BlockArg, InstBuilder};
use kata_ast::Spanned;
use kata_inference::{
    TypedExpr, TypedGuardClause, TypedLambdaClause, TypedPattern, TypedWithBinding,
};

use super::expr::lower_expr;
use super::pattern::test_clause_patterns;
use super::LowerCtx;
use crate::ffi_sigs::ty_to_clif;

/// Verifica se todos os patterns são `Ident` (binding simples).
pub(crate) fn all_patterns_are_ident(patterns: &[Spanned<TypedPattern>]) -> bool {
    patterns
        .iter()
        .all(|p| matches!(p.node, TypedPattern::Ident { .. }))
}

/// Bindar patterns Ident aos block params (parâmetros da função).
pub(crate) fn bind_patterns_to_params(
    patterns: &[Spanned<TypedPattern>],
    params: &[cranelift_codegen::ir::Value],
    lower: &mut LowerCtx,
) {
    for (pat, val) in patterns.iter().zip(params.iter()) {
        if let TypedPattern::Ident { name, ty } = &pat.node {
            let clif_ty = ty_to_clif(ty);
            let var = lower.new_var(name, clif_ty);
            lower.builder.def_var(var, *val);
        }
    }
}

/// Lowera o body de uma cláusula (com ou sem guards).
pub(crate) fn lower_clause_body(
    clause: &TypedLambdaClause,
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    if clause.guards.is_empty() {
        lower_expr(&clause.body.node, lower)
    } else {
        lower_guards(&clause.guards, &clause.body, lower)
    }
}

/// Lowera guards como branch chain dentro de uma cláusula.
pub(crate) fn lower_guards(
    guards: &[TypedGuardClause],
    fallback_body: &Spanned<TypedExpr>,
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let ret_clif = ty_to_clif(&fallback_body.node.ty);

    let cont_block = lower.builder.create_block();
    lower.builder.append_block_param(cont_block, ret_clif);

    let mut next_test_block = lower.builder.create_block();

    // Após o entry, jump para o primeiro teste de guard.
    lower.builder.ins().jump(next_test_block, &[]);
    // Agora que o único predecessor foi emitido, selar.
    lower.builder.seal_block(next_test_block);

    let mut had_otherwise = false;

    for (i, guard) in guards.iter().enumerate() {
        lower.builder.switch_to_block(next_test_block);

        let body_block = lower.builder.create_block();

        if let Some(cond) = &guard.condition {
            // Guard com condição: avalia condição → brif.
            let cond_val = lower_expr(&cond.node, lower)?;
            // Boolean é i64 (0 ou 1). Compara com 0 via icmp_imm (imediato).
            let is_true = lower.builder.ins().icmp_imm(
                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                cond_val,
                0,
            );
            // Próximo teste de guard (ou fallback).
            let next = if i + 1 < guards.len() {
                lower.builder.create_block()
            } else {
                // Fallback: body da cláusula (sem condição = otherwise).
                lower.builder.create_block()
            };
            lower
                .builder
                .ins()
                .brif(is_true, body_block, &[], next, &[]);
            // next_test_block já está selado (linha 499 na 1ª iteração,
            // ou foi selado como `next` na iteração anterior). NÃO re-selar.
            // Selar `next` (predecessor = este brif).
            lower.builder.seal_block(next);

            // Lowera o body do guard.
            lower.builder.switch_to_block(body_block);
            lower.builder.seal_block(body_block);
            let body_val = lower_expr(&guard.body.node, lower)?;
            lower
                .builder
                .ins()
                .jump(cont_block, &[BlockArg::Value(body_val)]);

            next_test_block = next;
        } else {
            // otherwise (sem condição): jump incondicional para o body.
            lower.builder.ins().jump(body_block, &[]);
            // next_test_block já foi selado no if let Some(cond) da iteração anterior.

            lower.builder.switch_to_block(body_block);
            lower.builder.seal_block(body_block);
            let body_val = lower_expr(&guard.body.node, lower)?;
            lower
                .builder
                .ins()
                .jump(cont_block, &[BlockArg::Value(body_val)]);

            // Não há próximo guard.
            had_otherwise = true;
            break;
        }
    }

    // Fallback: se nenhum guard passou e NÃO houve otherwise,
    // lowera o body da cláusula como fallback.
    // (Se houve otherwise, o next_test_block final já tem terminador.)
    if !had_otherwise {
        lower.builder.switch_to_block(next_test_block);
        let fallback_val = lower_expr(&fallback_body.node, lower)?;
        lower
            .builder
            .ins()
            .jump(cont_block, &[BlockArg::Value(fallback_val)]);
    }
    // next_test_block já foi selado dentro do loop (ou é o fallback block).
    lower.builder.seal_block(cont_block);
    lower.builder.switch_to_block(cont_block);
    let result = lower.builder.block_params(cont_block)[0];
    Ok(result)
}

/// Lowera múltiplas cláusulas como branch chain (pattern matching).
pub(crate) fn lower_clause_chain(
    clauses: &[TypedLambdaClause],
    params: &[cranelift_codegen::ir::Value],
    lower: &mut LowerCtx,
) -> Result<(), super::CodegenError> {
    let mut next_clause_block = lower.builder.create_block();
    lower.builder.ins().jump(next_clause_block, &[]);
    lower.builder.seal_block(next_clause_block);

    for clause in clauses {
        lower.builder.switch_to_block(next_clause_block);

        let body_block = lower.builder.create_block();

        // Testa o pattern da cláusula.
        let matches = test_clause_patterns(&clause.patterns, params, lower)?;

        // Cria o próximo block de teste (próxima cláusula) — sem selar ainda.
        next_clause_block = lower.builder.create_block();

        if let Some(cond_val) = matches {
            // Pattern com teste condicional (Literal/Variant): brif.
            lower
                .builder
                .ins()
                .brif(cond_val, body_block, &[], next_clause_block, &[]);
        } else {
            // Pattern incondicional (Ident/Wildcard): jump direto para o body.
            // Bindar e pular.
            bind_patterns_to_params(&clause.patterns, params, lower);
            lower.builder.ins().jump(body_block, &[]);
        }
        // Agora que os predecessores foram emitidos, selar.
        lower.builder.seal_block(next_clause_block);
        lower.builder.seal_block(body_block);

        // Switch para body_block antes de lowerar (o block atual já tem terminador).
        lower.builder.switch_to_block(body_block);

        // Lowera with bindings.
        lower_with_bindings(&clause.with_bindings, lower)?;

        // Lowera o body (com ou sem guards).
        // Se o body é um tail call (return_call), lower_expr emite return_call
        // e seta emitted_tail_call — não emitir return_ depois.
        lower.emitted_tail_call = false;
        let body_val = lower_clause_body(clause, lower)?;
        if !lower.emitted_tail_call {
            lower.builder.ins().return_(&[body_val]);
        }
    }

    // Nenhuma cláusula encaixou — runtime trap (não deveria acontecer se
    // o typeck verificou exaustividade).
    lower.builder.switch_to_block(next_clause_block);
    lower
        .builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"));
    // next_clause_block já foi selado dentro do loop.

    Ok(())
}

/// Lowera with bindings (computações prévias).
pub(crate) fn lower_with_bindings(
    with_bindings: &[TypedWithBinding],
    lower: &mut LowerCtx,
) -> Result<(), super::CodegenError> {
    for wb in with_bindings {
        let val = lower_expr(&wb.value.node, lower)?;
        let clif_ty = ty_to_clif(&wb.value.node.ty);
        let var = lower.new_var(&wb.name, clif_ty);
        lower.builder.def_var(var, val);
    }
    Ok(())
}