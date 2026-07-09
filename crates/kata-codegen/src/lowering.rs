//! Lowering TAST → CLIF (Cranelift IR).
//!
//! Percorre a `TypedModule` (TAST) e gera código nativo via `FunctionBuilder`.
//! Sem IR intermediária própria — o lowering é direto TAST → CLIF.
//!
//! Block arguments nativos (Cranelift 0.133) — sem stack slots.
//! Variáveis locais (`let`) usam `Variable` + `def_var`/`use_var` do builder.
//!
//! Fio 2 Fase 9: lowera funções nomeadas (múltiplas cláusulas), lambdas
//! anônimos, match, e call_indirect para lambdas como valores.

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, BlockArg, GlobalValueData, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ffi::FfiSymbol;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{
    TypedExpr, TypedExprKind, TypedFunction, TypedGuardClause, TypedLambdaClause, TypedModule,
    TypedPattern,
};
use kata_rt as rt;

use crate::ffi_sigs::ty_to_clif;
use crate::metadata::MetadataTable;

/// Erro de codegen.
#[derive(Debug)]
pub enum CodegenError {
    /// Símbolo FFI não encontrado no runtime.
    FfiSymbolNotFound(String),
    /// Erro interno do Cranelift.
    Cranelift(String),
    /// Nó da TAST não suportado neste lowering.
    UnsupportedNode(String),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::FfiSymbolNotFound(s) => write!(f, "símbolo FFI não encontrado: {s}"),
            CodegenError::Cranelift(s) => write!(f, "erro Cranelift: {s}"),
            CodegenError::UnsupportedNode(s) => write!(f, "nó TAST não suportado: {s}"),
        }
    }
}

impl std::error::Error for CodegenError {}

/// Registro de símbolos FFI no JITBuilder.
///
/// Cada símbolo do `FfiSymbol` enum é registrado com o ponteiro da função
/// C correspondente no `kata-rt`. O JIT usa esta tabela para resolver
/// imports.
pub fn register_ffi_symbols(builder: &mut cranelift_jit::JITBuilder) {
    // BigInt
    builder.symbol("kata_rt_bi_add", rt::kata_rt_bi_add as *const u8);
    builder.symbol("kata_rt_bi_sub", rt::kata_rt_bi_sub as *const u8);
    builder.symbol("kata_rt_bi_mul", rt::kata_rt_bi_mul as *const u8);
    builder.symbol("kata_rt_bi_div", rt::kata_rt_bi_div as *const u8);
    builder.symbol("kata_rt_bi_eq", rt::kata_rt_bi_eq as *const u8);
    builder.symbol("kata_rt_bi_neq", rt::kata_rt_bi_neq as *const u8);
    builder.symbol("kata_rt_bi_lt", rt::kata_rt_bi_lt as *const u8);
    builder.symbol("kata_rt_bi_le", rt::kata_rt_bi_le as *const u8);
    builder.symbol("kata_rt_bi_gt", rt::kata_rt_bi_gt as *const u8);
    builder.symbol("kata_rt_bi_ge", rt::kata_rt_bi_ge as *const u8);
    builder.symbol("kata_rt_bi_show", rt::kata_rt_bi_show as *const u8);
    builder.symbol(
        "kata_rt_bi_to_rational",
        rt::kata_rt_bi_to_rational as *const u8,
    );
    builder.symbol("kata_rt_tag_int", rt::kata_rt_tag_int as *const u8);
    builder.symbol(
        "kata_rt_tag_int_from_str",
        rt::kata_rt_tag_int_from_str as *const u8,
    );
    builder.symbol("kata_rt_int_to_text", rt::kata_rt_int_to_text as *const u8);
    // Float
    builder.symbol("kata_rt_fadd", rt::kata_rt_fadd as *const u8);
    builder.symbol("kata_rt_fsub", rt::kata_rt_fsub as *const u8);
    builder.symbol("kata_rt_fmul", rt::kata_rt_fmul as *const u8);
    builder.symbol("kata_rt_fdiv", rt::kata_rt_fdiv as *const u8);
    builder.symbol("kata_rt_fcmp_eq", rt::kata_rt_fcmp_eq as *const u8);
    builder.symbol("kata_rt_fcmp_neq", rt::kata_rt_fcmp_neq as *const u8);
    builder.symbol("kata_rt_fcmp_lt", rt::kata_rt_fcmp_lt as *const u8);
    builder.symbol("kata_rt_fcmp_le", rt::kata_rt_fcmp_le as *const u8);
    builder.symbol("kata_rt_fcmp_gt", rt::kata_rt_fcmp_gt as *const u8);
    builder.symbol("kata_rt_fcmp_ge", rt::kata_rt_fcmp_ge as *const u8);
    // Rational
    builder.symbol("kata_rt_rat_add", rt::kata_rt_rat_add as *const u8);
    builder.symbol("kata_rt_rat_sub", rt::kata_rt_rat_sub as *const u8);
    builder.symbol("kata_rt_rat_mul", rt::kata_rt_rat_mul as *const u8);
    builder.symbol("kata_rt_rat_div", rt::kata_rt_rat_div as *const u8);
    builder.symbol("kata_rt_rat_eq", rt::kata_rt_rat_eq as *const u8);
    builder.symbol("kata_rt_rat_neq", rt::kata_rt_rat_neq as *const u8);
    builder.symbol("kata_rt_rat_lt", rt::kata_rt_rat_lt as *const u8);
    builder.symbol("kata_rt_rat_le", rt::kata_rt_rat_le as *const u8);
    builder.symbol("kata_rt_rat_gt", rt::kata_rt_rat_gt as *const u8);
    builder.symbol("kata_rt_rat_ge", rt::kata_rt_rat_ge as *const u8);
    builder.symbol("kata_rt_rat_show", rt::kata_rt_rat_show as *const u8);
    builder.symbol(
        "kata_rt_rat_to_float",
        rt::kata_rt_rat_to_float as *const u8,
    );
    builder.symbol(
        "kata_rt_rat_from_float",
        rt::kata_rt_rat_from_float as *const u8,
    );
    builder.symbol("kata_rt_rat_literal", rt::kata_rt_rat_literal as *const u8);
    builder.symbol(
        "kata_rt_int_to_rational",
        rt::kata_rt_int_to_rational as *const u8,
    );
    // Text
    builder.symbol(
        "kata_rt_string_concat",
        rt::kata_rt_string_concat as *const u8,
    );
    builder.symbol("kata_rt_string_len", rt::kata_rt_string_len as *const u8);
    builder.symbol(
        "kata_rt_text_literal",
        rt::kata_rt_text_literal as *const u8,
    );
    builder.symbol(
        "kata_rt_bool_to_text",
        rt::kata_rt_bool_to_text as *const u8,
    );
    builder.symbol(
        "kata_rt_text_replace_first",
        rt::kata_rt_text_replace_first as *const u8,
    );
    // I/O
    builder.symbol("kata_rt_print", rt::kata_rt_print as *const u8);
    builder.symbol("kata_rt_println", rt::kata_rt_println as *const u8);
    // Arena — não exportado como C-ABI em Fio 1; registrar em fios posteriores.
}

/// Declara todos os símbolos FFI no module e retorna o mapa nome → FuncId.
pub fn declare_ffi_symbols(
    module: &mut cranelift_jit::JITModule,
) -> Result<HashMap<String, cranelift_module::FuncId>, CodegenError> {
    let mut ffi_ids = HashMap::new();
    for sym in all_ffi_symbols() {
        let name = sym.symbol_name();
        let sig = crate::ffi_sigs::ffi_signature(sym);
        let fid = module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| CodegenError::Cranelift(format!("declare FFI {name}: {e}")))?;
        ffi_ids.insert(name.to_string(), fid);
    }
    // Símbolo especial: kata_rt_tag_int_from_str (não está no FfiSymbol enum).
    // Usado para lowerar IntLit que não cabe em SMI (BigInts).
    let tag_str_sig = {
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(I64)); // ptr
        sig.params.push(AbiParam::new(I64)); // len
        sig.returns.push(AbiParam::new(I64)); // tagged i64
        sig
    };
    let tag_str_fid = module
        .declare_function("kata_rt_tag_int_from_str", Linkage::Import, &tag_str_sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare kata_rt_tag_int_from_str: {e}")))?;
    ffi_ids.insert("kata_rt_tag_int_from_str".to_string(), tag_str_fid);
    Ok(ffi_ids)
}

/// Todos os símbolos FFI que o codegen conhece.
fn all_ffi_symbols() -> Vec<FfiSymbol> {
    use FfiSymbol::*;
    vec![
        BiAdd,
        BiSub,
        BiMul,
        BiDiv,
        BiEq,
        BiNeq,
        BiLt,
        BiLe,
        BiGt,
        BiGe,
        BiShow,
        BiToRational,
        TagInt,
        IntToText,
        Fadd,
        Fsub,
        Fmul,
        Fdiv,
        FcmpEq,
        FcmpNeq,
        FcmpLt,
        FcmpLe,
        FcmpGt,
        FcmpGe,
        RatAdd,
        RatSub,
        RatMul,
        RatDiv,
        RatEq,
        RatNeq,
        RatLt,
        RatLe,
        RatGt,
        RatGe,
        RatShow,
        RatToFloat,
        RatFromFloat,
        RatLiteral,
        IntToRational,
        StringConcat,
        StringLen,
        TextLiteral,
        BoolToText,
        TextReplaceFirst,
        Print,
        Println,
    ]
}

/// Tabela de strings literais — indexada por índice.
pub type StringTable = Vec<String>;

/// Tabela de símbolos de funções Kata nomeadas — mapeia nome → FuncId.
type SymbolTable = HashMap<String, cranelift_module::FuncId>;

/// Lower do `TypedModule` completo: cria a função `__kata_entry` e
/// retorna o `MetadataTable` sidecar + a string table.
pub fn lower_module(
    typed: &TypedModule,
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
) -> Result<(MetadataTable, StringTable), CodegenError> {
    let mut metadata = MetadataTable::new();
    let mut string_table = StringTable::new();
    let mut symbol_table: SymbolTable = HashMap::new();

    // ── Fase 9: declara e define funções nomeadas antes do entry point ──
    for func in &typed.functions {
        let func_id = declare_kata_function(func, module)?;
        symbol_table.insert(func.name.clone(), func_id);
    }

    for func in &typed.functions {
        define_kata_function(func, module, ffi_ids, &symbol_table, &mut string_table)?;
    }

    // Determina o tipo de retorno do entry point.
    let ret_ty = &typed.entry.node.ty;
    let ret_clif = ty_to_clif(ret_ty);

    // Assinatura do __kata_entry: () → ret_clif
    let mut sig = Signature::new(CallConv::SystemV);
    sig.returns.push(AbiParam::new(ret_clif));

    let entry_id = module
        .declare_function("__kata_entry", Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare __kata_entry: {e}")))?;

    // Cria um Context do Cranelift (não FunctionBuilderContext).
    let mut ctx = module.make_context();

    // Constrói a função IR dentro do Context.
    {
        let func = &mut ctx.func;
        func.signature = sig.clone();

        // Declara cada FFI no Function e coleta os FuncRefs.
        let mut ffi_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (name, &fid) in ffi_ids {
            let func_ref = module.declare_func_in_func(fid, func);
            ffi_refs.insert(name.clone(), func_ref);
        }

        // Declara funções Kata nomeadas no Function (para call direto).
        let mut kata_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (name, &fid) in &symbol_table {
            let func_ref = module.declare_func_in_func(fid, func);
            kata_refs.insert(name.clone(), func_ref);
        }

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(func, &mut func_ctx);

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let mut lower = LowerCtx {
            builder: &mut builder,
            module,
            ffi_refs: &ffi_refs,
            kata_refs: &kata_refs,
            ffi_ids,
            kata_ids: &symbol_table,
            metadata: &mut metadata,
            string_table: &mut string_table,
            var_map: HashMap::new(),
            anon_counter: 0,
        };

        // Lowera pre_entry (let bindings e outras expressões top-level anteriores).
        // Estas são loweradas em sequência, compartilhando o var_map —
        // um `let` define uma variável que o entry pode usar.
        for pre in &typed.pre_entry {
            lower_expr(&pre.node, &mut lower)?;
        }

        let result = lower_expr(&typed.entry.node, &mut lower)?;

        // Return.
        match ret_ty {
            Ty::Unit => {
                let zero = lower.builder.ins().iconst(I64, 0);
                lower.builder.ins().return_(&[zero]);
            }
            _ => {
                lower.builder.ins().return_(&[result]);
            }
        }

        builder.finalize();
    }

    // Define a função no module usando o Context.
    module
        .define_function(entry_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift(format!("define __kata_entry: {e}")))?;

    module.clear_context(&mut ctx);

    // Define os data symbols para strings literais.
    for (i, s) in string_table.iter().enumerate() {
        let sym = format!("__kata_str_{i}");
        let did = module
            .declare_data(&sym, Linkage::Local, false, false)
            .map_err(|e| CodegenError::Cranelift(format!("declare_data {sym}: {e}")))?;
        let mut data_desc = cranelift_module::DataDescription::new();
        data_desc.define(s.as_bytes().to_vec().into());
        module
            .define_data(did, &data_desc)
            .map_err(|e| CodegenError::Cranelift(format!("define_data {sym}: {e}")))?;
    }

    Ok((metadata, string_table))
}

/// Declara uma função Kata nomeada no JITModule (sem definir ainda).
fn declare_kata_function(
    func: &TypedFunction,
    module: &mut cranelift_jit::JITModule,
) -> Result<cranelift_module::FuncId, CodegenError> {
    let mut sig = Signature::new(CallConv::SystemV);
    for pt in &func.param_types {
        sig.params.push(AbiParam::new(ty_to_clif(pt)));
    }
    sig.returns.push(AbiParam::new(ty_to_clif(&func.ret_ty)));
    module
        .declare_function(&func.name, Linkage::Export, &sig)
        .map_err(|e| CodegenError::Cranelift(format!("declare kata fn {}: {e}", func.name)))
}

/// Define (compila o corpo de) uma função Kata nomeada.
fn define_kata_function(
    func: &TypedFunction,
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
    symbol_table: &SymbolTable,
    string_table: &mut StringTable,
) -> Result<(), CodegenError> {
    let mut ctx = module.make_context();
    let mut metadata = MetadataTable::new();

    // Constrói a assinatura.
    {
        let func_ir = &mut ctx.func;
        let mut sig = Signature::new(CallConv::SystemV);
        for pt in &func.param_types {
            sig.params.push(AbiParam::new(ty_to_clif(pt)));
        }
        sig.returns.push(AbiParam::new(ty_to_clif(&func.ret_ty)));
        func_ir.signature = sig;

        // Declara FFI e funções Kata no Function.
        let mut ffi_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (name, &fid) in ffi_ids {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            ffi_refs.insert(name.clone(), func_ref);
        }
        let mut kata_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (name, &fid) in symbol_table {
            let func_ref = module.declare_func_in_func(fid, func_ir);
            kata_refs.insert(name.clone(), func_ref);
        }

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(func_ir, &mut func_ctx);

        let entry_block = builder.create_block();
        // Parâmetros da função viram block params do entry block.
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        // Coleta os block params (parâmetros da função) e mapeia para nomes
        // extraídos dos patterns da primeira cláusula.
        let params: Vec<cranelift_codegen::ir::Value> = builder.block_params(entry_block).to_vec();

        let mut lower = LowerCtx {
            builder: &mut builder,
            module,
            ffi_refs: &ffi_refs,
            kata_refs: &kata_refs,
            ffi_ids,
            kata_ids: symbol_table,
            metadata: &mut metadata,
            string_table,
            var_map: HashMap::new(),
            anon_counter: 0,
        };

        // Para uma única cláusula com patterns Ident: bindar params diretamente.
        // Para múltiplas cláusulas: branch chain com pattern tests.
        if func.clauses.len() == 1 && all_patterns_are_ident(&func.clauses[0].patterns) {
            // Caso simples: 1 cláusula, todos patterns são Ident.
            let clause = &func.clauses[0];
            bind_patterns_to_params(&clause.patterns, &params, &mut lower);
            // Lowerar with bindings.
            lower_with_bindings(&clause.with_bindings, &mut lower)?;
            // Lowerar body (ou guards).
            let result = lower_clause_body(clause, &mut lower)?;
            lower.builder.ins().return_(&[result]);
        } else {
            // Múltiplas cláusulas: branch chain.
            let result = lower_clause_chain(&func.clauses, &params, &mut lower)?;
            lower.builder.ins().return_(&[result]);
        }

        builder.finalize();
    }

    // Define a função no module.
    let func_id = *symbol_table.get(&func.name).ok_or_else(|| {
        CodegenError::Cranelift(format!("func {} not in symbol_table", func.name))
    })?;
    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Cranelift(format!("define kata fn {}: {e}", func.name)))?;
    module.clear_context(&mut ctx);
    Ok(())
}

/// Verifica se todos os patterns são `Ident` (binding simples).
fn all_patterns_are_ident(patterns: &[kata_ast::Spanned<TypedPattern>]) -> bool {
    patterns
        .iter()
        .all(|p| matches!(p.node, TypedPattern::Ident { .. }))
}

/// Bindar patterns Ident aos block params (parâmetros da função).
fn bind_patterns_to_params(
    patterns: &[kata_ast::Spanned<TypedPattern>],
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
fn lower_clause_body(
    clause: &TypedLambdaClause,
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    if clause.guards.is_empty() {
        lower_expr(&clause.body.node, lower)
    } else {
        lower_guards(&clause.guards, &clause.body, lower)
    }
}

/// Lowera guards como branch chain dentro de uma cláusula.
fn lower_guards(
    guards: &[TypedGuardClause],
    fallback_body: &kata_ast::Spanned<TypedExpr>,
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
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
            // Boolean é i64 (0 ou 1). Compara com 0 (false).
            let _zero = lower.builder.ins().iconst(I64, 0);
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
fn lower_clause_chain(
    clauses: &[TypedLambdaClause],
    params: &[cranelift_codegen::ir::Value],
    lower: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let ret_clif = ty_to_clif(&clauses[0].body.node.ty);

    let cont_block = lower.builder.create_block();
    lower.builder.append_block_param(cont_block, ret_clif);

    let mut next_clause_block = lower.builder.create_block();
    lower.builder.ins().jump(next_clause_block, &[]);
    lower.builder.seal_block(next_clause_block);

    for clause in clauses {
        lower.builder.switch_to_block(next_clause_block);

        let body_block = lower.builder.create_block();

        // Testa o pattern da cláusula.
        let matches = test_clause_patterns(&clause.patterns, params, lower, body_block)?;

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
        let body_val = lower_clause_body(clause, lower)?;
        lower
            .builder
            .ins()
            .jump(cont_block, &[BlockArg::Value(body_val)]);
    }

    // Nenhuma cláusula encaixou — runtime trap (não deveria acontecer se
    // o typeck verificou exaustividade).
    lower.builder.switch_to_block(next_clause_block);
    lower
        .builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::user(1).unwrap());
    // next_clause_block já foi selado dentro do loop.

    lower.builder.seal_block(cont_block);
    lower.builder.switch_to_block(cont_block);
    let result = lower.builder.block_params(cont_block)[0];
    Ok(result)
}

/// Testa patterns de uma cláusula contra os parâmetros.
/// Retorna `Some(cond_val)` se há um teste condicional (brif), ou `None`
/// se o pattern é incondicional (Ident/Wildcard — sempre encaixa).
fn test_clause_patterns(
    patterns: &[kata_ast::Spanned<TypedPattern>],
    params: &[cranelift_codegen::ir::Value],
    lower: &mut LowerCtx,
    _body_block: cranelift_codegen::ir::Block,
) -> Result<Option<cranelift_codegen::ir::Value>, CodegenError> {
    let mut all_matches = None;

    for (pat, val) in patterns.iter().zip(params.iter()) {
        match &pat.node {
            TypedPattern::Ident { name, ty } => {
                let clif_ty = ty_to_clif(ty);
                let var = lower.new_var(name, clif_ty);
                lower.builder.def_var(var, *val);
            }
            TypedPattern::Wildcard => {
                // Aceita qualquer valor — não binda.
            }
            TypedPattern::Literal { value } => {
                let lit_val = lower_expr(&value.node, lower)?;
                let eq = lower.builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    *val,
                    lit_val,
                );
                all_matches = Some(match all_matches {
                    None => eq,
                    Some(prev) => lower.builder.ins().band(prev, eq),
                });
            }
            TypedPattern::Variant { enum_name, variant } => {
                // Boolean: True = 1, False = 0.
                if enum_name == "Boolean" {
                    let expected = if variant == "True" { 1 } else { 0 };
                    let eq = lower.builder.ins().icmp_imm(
                        cranelift_codegen::ir::condcodes::IntCC::Equal,
                        *val,
                        expected,
                    );
                    all_matches = Some(match all_matches {
                        None => eq,
                        Some(prev) => lower.builder.ins().band(prev, eq),
                    });
                } else {
                    return Err(CodegenError::UnsupportedNode(format!(
                        "Pattern Variant não-Boolean: {enum_name}::{variant}"
                    )));
                }
            }
            TypedPattern::Tuple { .. } => {
                return Err(CodegenError::UnsupportedNode(
                    "Pattern Tuple em cláusula lambda: ainda não implementado".into(),
                ));
            }
            TypedPattern::Cons { .. } => {
                return Err(CodegenError::UnsupportedNode(
                    "Pattern Cons: List é Fio 8".into(),
                ));
            }
        }
    }

    Ok(all_matches)
}

/// Lowera with bindings (computações prévias).
fn lower_with_bindings(
    with_bindings: &[kata_inference::TypedWithBinding],
    lower: &mut LowerCtx,
) -> Result<(), CodegenError> {
    for wb in with_bindings {
        let val = lower_expr(&wb.value.node, lower)?;
        let clif_ty = ty_to_clif(&wb.value.node.ty);
        let var = lower.new_var(&wb.name, clif_ty);
        lower.builder.def_var(var, val);
    }
    Ok(())
}

/// Contexto de lowering — compartilhado entre as chamadas recursivas.
struct LowerCtx<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    module: &'a mut cranelift_jit::JITModule,
    ffi_refs: &'a HashMap<String, cranelift_codegen::ir::FuncRef>,
    kata_refs: &'a HashMap<String, cranelift_codegen::ir::FuncRef>,
    /// FuncIds globais (module-level) para re-declaração em lambdas anônimos.
    ffi_ids: &'a HashMap<String, cranelift_module::FuncId>,
    kata_ids: &'a HashMap<String, cranelift_module::FuncId>,
    #[allow(dead_code)]
    metadata: &'a mut MetadataTable,
    string_table: &'a mut StringTable,
    var_map: HashMap<String, cranelift_frontend::Variable>,
    anon_counter: u32,
}

impl<'a, 'b> LowerCtx<'a, 'b> {
    /// Declara uma nova variável no builder e mapeia o nome.
    fn new_var(
        &mut self,
        name: &str,
        ty: cranelift_codegen::ir::Type,
    ) -> cranelift_frontend::Variable {
        let var = self.builder.declare_var(ty);
        self.var_map.insert(name.to_string(), var);
        var
    }

    /// Adiciona uma string à string table e retorna o DataId + GlobalValue.
    /// O GlobalValue aponta para o endereço da string no module.
    fn add_string(&mut self, text: &str) -> cranelift_codegen::ir::GlobalValue {
        let idx = self.string_table.len();
        self.string_table.push(text.to_string());
        let sym = format!("__kata_str_{idx}");
        let did = self
            .module
            .declare_data(&sym, Linkage::Local, false, false)
            .expect("declare_data falhou para string literal");
        // declare_data_in_func cria o GlobalValue apontando para o data symbol.
        self.module.declare_data_in_func(did, self.builder.func)
    }

    /// Gera um nome fresh para lambda anônimo.
    fn fresh_anon_name(&mut self) -> String {
        let name = format!("__anon_{}", self.anon_counter);
        self.anon_counter += 1;
        name
    }
}

/// Lowera uma expressão TAST → valor CLIF.
fn lower_expr(
    expr: &TypedExpr,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    match &expr.kind {
        // ── IntLit: SMI inline ou kata_rt_tag_int_from_str para BigInt ──
        TypedExprKind::IntLit { text } => {
            // Tenta parsear como i64. Se conseguir e couber em SMI, inline.
            // Se não (BigInt grande), chama kata_rt_tag_int_from_str.
            let parsed = parse_int_literal(text);
            if let Some(val) = parsed {
                if fits_smi(val) {
                    // SMI: (val << 1) | 1 — inline, sem FFI call.
                    return Ok(ctx.builder.ins().iconst(I64, encode_smi(val)));
                }
                // i64 mas não cabe em SMI — chama kata_rt_tag_int(val).
                let raw = ctx.builder.ins().iconst(I64, val);
                let func_ref = ctx
                    .ffi_refs
                    .get("kata_rt_tag_int")
                    .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_tag_int".into()))?;
                let call_inst = ctx.builder.ins().call(*func_ref, &[raw]);
                return Ok(ctx.builder.inst_results(call_inst)[0]);
            }
            // BigInt grande: chama kata_rt_tag_int_from_str(ptr, len).
            let global = ctx.add_string(text);
            let ptr = ctx
                .builder
                .ins()
                .global_value(ctx.module.target_config().pointer_type(), global);
            let len = ctx.builder.ins().iconst(I64, text.len() as i64);
            let func_ref = ctx
                .ffi_refs
                .get("kata_rt_tag_int_from_str")
                .ok_or_else(|| {
                    CodegenError::FfiSymbolNotFound("kata_rt_tag_int_from_str".into())
                })?;
            let call_inst = ctx.builder.ins().call(*func_ref, &[ptr, len]);
            Ok(ctx.builder.inst_results(call_inst)[0])
        }

        // ── FloatLit: f64 const direto ──
        TypedExprKind::FloatLit { text } => {
            let val: f64 = text.parse().unwrap_or(f64::NAN);
            Ok(ctx.builder.ins().f64const(val))
        }

        // ── TextLit: string alocada como data symbol ──
        TypedExprKind::TextLit { text } => {
            let global = ctx.add_string(text);
            let ptr = ctx
                .builder
                .ins()
                .global_value(ctx.module.target_config().pointer_type(), global);
            Ok(ptr)
        }

        // ── Unit: i64 zero ──
        TypedExprKind::Unit => Ok(ctx.builder.ins().iconst(I64, 0)),

        // ── Ident: use_var ──
        TypedExprKind::Ident { name } => {
            let var = ctx
                .var_map
                .get(name)
                .ok_or_else(|| CodegenError::UnsupportedNode(format!("unbound ident: {name}")))?;
            Ok(ctx.builder.use_var(*var))
        }

        // ── Closure: call FFI, call direto (Kata), ou call_indirect ──
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
            captures: _,
            escapes: _,
        } => {
            // Lowera os argumentos.
            let mut arg_values = Vec::with_capacity(args.len());
            for arg in args {
                let val = lower_expr(&arg.node, ctx)?;
                arg_values.push(val);
            }

            if let Some(sym_name) = ffi_symbol {
                // Call FFI direto.
                let func_ref = ctx
                    .ffi_refs
                    .get(sym_name)
                    .ok_or_else(|| CodegenError::FfiSymbolNotFound(sym_name.clone()))?;
                let call_inst = ctx.builder.ins().call(*func_ref, &arg_values);
                Ok(ctx.builder.inst_results(call_inst)[0])
            } else {
                // ffi_symbol = None: função Kata nomeada ou lambda como valor.
                // Tenta Kata function call direto primeiro.
                if let TypedExprKind::Ident { name } = &callee.node.kind {
                    if let Some(&func_ref) = ctx.kata_refs.get(name) {
                        // Call direto para função Kata nomeada.
                        let call_inst = ctx.builder.ins().call(func_ref, &arg_values);
                        return Ok(ctx.builder.inst_results(call_inst)[0]);
                    }
                    // Ident não está no kata_refs: pode ser variável com
                    // Ty::Function (lambda como valor) — call_indirect.
                    if let Some(var) = ctx.var_map.get(name) {
                        let func_ptr = ctx.builder.use_var(*var);
                        // Constrói a assinatura para call_indirect.
                        // O tipo do callee é Ty::Function(params, ret).
                        let callee_ty = &callee.node.ty;
                        if let Ty::Function(param_types, ret_ty) = callee_ty {
                            let mut sig = Signature::new(CallConv::SystemV);
                            for pt in param_types {
                                sig.params.push(AbiParam::new(ty_to_clif(pt)));
                            }
                            sig.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
                            let sig_ref = ctx.builder.func.import_signature(sig);
                            let call_inst =
                                ctx.builder
                                    .ins()
                                    .call_indirect(sig_ref, func_ptr, &arg_values);
                            return Ok(ctx.builder.inst_results(call_inst)[0]);
                        }
                    }
                }
                Err(CodegenError::UnsupportedNode(format!(
                    "Closure sem ffi_symbol e callee não-Ident: {:?}",
                    callee.node.kind
                )))
            }
        }

        // ── TypeAscription: inspeciona (inner.kind, target_ty) ──
        TypedExprKind::TypeAscription { expr, target_ty } => {
            let inner = &expr.node;
            match (&inner.kind, target_ty) {
                // IntLit → Float: reinterpretar como f64 const.
                (TypedExprKind::IntLit { text }, Ty::Prim(PrimTy::Float)) => {
                    let val: f64 = text.parse().unwrap_or(f64::NAN);
                    Ok(ctx.builder.ins().f64const(val))
                }
                // IntLit → Rational: chama kata_rt_rat_literal.
                (TypedExprKind::IntLit { text }, Ty::Prim(PrimTy::Rational)) => {
                    let global = ctx.add_string(text);
                    let ptr = ctx
                        .builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), global);
                    let len = ctx.builder.ins().iconst(I64, text.len() as i64);
                    let func_ref = ctx.ffi_refs.get("kata_rt_rat_literal").ok_or_else(|| {
                        CodegenError::FfiSymbolNotFound("kata_rt_rat_literal".into())
                    })?;
                    let call_inst = ctx.builder.ins().call(*func_ref, &[ptr, len]);
                    Ok(ctx.builder.inst_results(call_inst)[0])
                }
                // FloatLit → Rational: chama kata_rt_rat_literal.
                (TypedExprKind::FloatLit { text }, Ty::Prim(PrimTy::Rational)) => {
                    let global = ctx.add_string(text);
                    let ptr = ctx
                        .builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), global);
                    let len = ctx.builder.ins().iconst(I64, text.len() as i64);
                    let func_ref = ctx.ffi_refs.get("kata_rt_rat_literal").ok_or_else(|| {
                        CodegenError::FfiSymbolNotFound("kata_rt_rat_literal".into())
                    })?;
                    let call_inst = ctx.builder.ins().call(*func_ref, &[ptr, len]);
                    Ok(ctx.builder.inst_results(call_inst)[0])
                }
                // Mesmo tipo (no-op): lowerar inner.
                _ if inner.ty == *target_ty => lower_expr(inner, ctx),
                // Demais casos: o typeck já deveria ter rejeitado.
                _ => Err(CodegenError::UnsupportedNode(format!(
                    "ascription não suportada: {:?} → {:?}",
                    inner.kind, target_ty
                ))),
            }
        }

        // ── Grouping: transparente ──
        TypedExprKind::Grouping { inner } => lower_expr(&inner.node, ctx),

        // ── Tuple: não suportado em Fio 2 ──
        TypedExprKind::Tuple { .. } => Err(CodegenError::UnsupportedNode(
            "tuple não suportado em Fio 2 codegen".into(),
        )),

        // ── Let: define variável ──
        TypedExprKind::Let { name, value } => {
            let val = lower_expr(&value.node, ctx)?;
            let clif_ty = ty_to_clif(&value.node.ty);
            let var = ctx.new_var(name, clif_ty);
            ctx.builder.def_var(var, val);
            // Let retorna Unit.
            Ok(ctx.builder.ins().iconst(I64, 0))
        }

        // ── VariantQual: Boolean::True = 1, Boolean::False = 0 ──
        TypedExprKind::VariantQual { enum_name, variant } => {
            if enum_name == "Boolean" {
                let val = if variant == "True" { 1 } else { 0 };
                Ok(ctx.builder.ins().iconst(I64, val))
            } else {
                Err(CodegenError::UnsupportedNode(format!(
                    "VariantQual não suportado: {enum_name}::{variant}"
                )))
            }
        }

        // ── Lambda: função Cranelift separada (anon) ou referência (nomeado) ──
        TypedExprKind::Lambda {
            func_name,
            param_types,
            ret_ty,
            clauses,
        } => {
            // Para lambda anônimo (func_name = None): declarar e definir
            // uma função separada no JITModule.
            // Para função nomeada (func_name = Some): já foi definida em
            // lower_module — retornar o function pointer.
            let name = match func_name {
                Some(n) => n.clone(),
                None => ctx.fresh_anon_name(),
            };

            // Declara a função anônima.
            let mut sig = Signature::new(CallConv::SystemV);
            for pt in param_types {
                sig.params.push(AbiParam::new(ty_to_clif(pt)));
            }
            sig.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
            let func_id = ctx
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| CodegenError::Cranelift(format!("declare anon fn {name}: {e}")))?;

            // Define a função anônima inline.
            // NOTA: isto é uma simplificação — em produção, lambdas anônimos
            // deveriam ser compilados antes do entry point (como funções
            // nomeadas). Por enquanto, compilamos inline e usamos
            // finalize_definitions no jit_eval para resolver.
            // O Cranelift JIT requer que todas as funções sejam definidas
            // antes de finalize_definitions. Isto funciona porque jit_eval
            // chama finalize_definitions depois de lower_module.
            {
                let mut ctx2 = ctx.module.make_context();
                let func_ir = &mut ctx2.func;
                let mut sig2 = Signature::new(CallConv::SystemV);
                for pt in param_types {
                    sig2.params.push(AbiParam::new(ty_to_clif(pt)));
                }
                sig2.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
                func_ir.signature = sig2;

                let mut ffi_refs2: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
                for (fname, &fid) in ctx.ffi_ids {
                    let func_ref = ctx.module.declare_func_in_func(fid, func_ir);
                    ffi_refs2.insert(fname.clone(), func_ref);
                }

                let mut kata_refs2: HashMap<String, cranelift_codegen::ir::FuncRef> =
                    HashMap::new();
                for (fname, &fid) in ctx.kata_ids {
                    let func_ref = ctx.module.declare_func_in_func(fid, func_ir);
                    kata_refs2.insert(fname.clone(), func_ref);
                }

                let mut func_ctx2 = FunctionBuilderContext::new();
                let mut builder2 = FunctionBuilder::new(func_ir, &mut func_ctx2);

                let entry_block2 = builder2.create_block();
                builder2.append_block_params_for_function_params(entry_block2);
                builder2.switch_to_block(entry_block2);
                builder2.seal_block(entry_block2);

                let params: Vec<cranelift_codegen::ir::Value> =
                    builder2.block_params(entry_block2).to_vec();

                let mut lower2 = LowerCtx {
                    builder: &mut builder2,
                    module: ctx.module,
                    ffi_refs: &ffi_refs2,
                    kata_refs: &kata_refs2,
                    ffi_ids: ctx.ffi_ids,
                    kata_ids: ctx.kata_ids,
                    metadata: ctx.metadata,
                    string_table: ctx.string_table,
                    var_map: HashMap::new(),
                    anon_counter: 0,
                };

                // Lowera as cláusulas do lambda.
                if clauses.len() == 1 && all_patterns_are_ident(&clauses[0].patterns) {
                    let clause = &clauses[0];
                    bind_patterns_to_params(&clause.patterns, &params, &mut lower2);
                    lower_with_bindings(&clause.with_bindings, &mut lower2)?;
                    let result = lower_clause_body(clause, &mut lower2)?;
                    lower2.builder.ins().return_(&[result]);
                } else {
                    let result = lower_clause_chain(clauses, &params, &mut lower2)?;
                    lower2.builder.ins().return_(&[result]);
                }

                builder2.finalize();
                ctx.module
                    .define_function(func_id, &mut ctx2)
                    .map_err(|e| CodegenError::Cranelift(format!("define anon fn {name}: {e}")))?;
                ctx.module.clear_context(&mut ctx2);
            }

            // Retorna o function pointer como valor.
            // Em Fio 2, funções Kata nomeadas atribuídas a variáveis usam
            // call_indirect. O function pointer é obtido via global_value.
            // declare_func_in_func retorna FuncRef (para call direto);
            // para call_indirect precisamos de um GlobalValue apontando
            // para o símbolo da função.
            let func_ref = ctx.module.declare_func_in_func(func_id, ctx.builder.func);
            let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
            let func_gv = ctx
                .builder
                .func
                .create_global_value(GlobalValueData::Symbol {
                    name: ext_func_name,
                    offset: 0.into(),
                    colocated: true,
                    tls: false,
                });
            let func_ptr = ctx
                .builder
                .ins()
                .global_value(ctx.module.target_config().pointer_type(), func_gv);
            Ok(func_ptr)
        }

        // ── Match: pattern matching com branch chain ──
        TypedExprKind::Match { scrutinee, arms } => lower_match(scrutinee, arms, ctx),
    }
}

/// Lowera um match: branch chain com brif para cada arm.
fn lower_match(
    scrutinee: &kata_ast::Spanned<TypedExpr>,
    arms: &[kata_inference::TypedMatchArm],
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let scrutinee_val = lower_expr(&scrutinee.node, ctx)?;
    let _ret_clif = ty_to_clif(&scrutinee.node.ty);
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
            match &pat.node {
                TypedPattern::Ident { name, ty } => {
                    let clif_ty = ty_to_clif(ty);
                    let var = ctx.new_var(name, clif_ty);
                    ctx.builder.def_var(var, scrutinee_val);
                    // Ident sempre encaixa.
                }
                TypedPattern::Wildcard => {
                    // Wildcard sempre encaixa.
                }
                TypedPattern::Literal { value } => {
                    let lit_val = lower_expr(&value.node, ctx)?;
                    let eq = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::Equal,
                        scrutinee_val,
                        lit_val,
                    );
                    pattern_cond = Some(eq);
                }
                TypedPattern::Variant { enum_name, variant } => {
                    if enum_name == "Boolean" {
                        let expected = if variant == "True" { 1 } else { 0 };
                        let eq = ctx.builder.ins().icmp_imm(
                            cranelift_codegen::ir::condcodes::IntCC::Equal,
                            scrutinee_val,
                            expected,
                        );
                        pattern_cond = Some(eq);
                    } else {
                        return Err(CodegenError::UnsupportedNode(format!(
                            "Match Variant não-Boolean: {enum_name}::{variant}"
                        )));
                    }
                }
                TypedPattern::Tuple { .. } => {
                    return Err(CodegenError::UnsupportedNode(
                        "Match Tuple: ainda não implementado".into(),
                    ));
                }
                TypedPattern::Cons { .. } => {
                    return Err(CodegenError::UnsupportedNode(
                        "Match Cons: List é Fio 8".into(),
                    ));
                }
            }
        } else {
            // otherwise (pattern = None): sempre encaixa.
        }

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
            let _zero = ctx.builder.ins().iconst(I64, 0);
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
            .jump(cont_block, &[BlockArg::Value(body_val)]);
    }

    // Nenhum arm encaixou — runtime trap.
    ctx.builder.switch_to_block(next_test_block);
    ctx.builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::user(1).unwrap());
    // next_test_block já foi selado dentro do loop.

    ctx.builder.seal_block(cont_block);
    ctx.builder.switch_to_block(cont_block);
    let result = ctx.builder.block_params(cont_block)[0];
    Ok(result)
}

// ── Helpers de SMI tagging (duplicados do runtime para uso em compile-time) ──

fn fits_smi(val: i64) -> bool {
    (-(1i64 << 62)..(1i64 << 62)).contains(&val)
}

fn encode_smi(val: i64) -> i64 {
    (val << 1) | 1
}

/// Parseia um literal inteiro (decimal/hex/oct/bin, com underscore).
/// Retorna None se o número não cabe em i64 (BigInt).
fn parse_int_literal(text: &str) -> Option<i64> {
    let cleaned = text.replace('_', "");
    let (sign, digits) = if let Some(rest) = cleaned.strip_prefix('-') {
        (-1i64, rest)
    } else if let Some(rest) = cleaned.strip_prefix('+') {
        (1i64, rest)
    } else {
        (1i64, cleaned.as_str())
    };

    let n = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16).ok()
    } else if let Some(oct) = digits
        .strip_prefix("0o")
        .or_else(|| digits.strip_prefix("0O"))
    {
        i64::from_str_radix(oct, 8).ok()
    } else if let Some(bin) = digits
        .strip_prefix("0b")
        .or_else(|| digits.strip_prefix("0B"))
    {
        i64::from_str_radix(bin, 2).ok()
    } else if let Some(dec) = digits
        .strip_prefix("0d")
        .or_else(|| digits.strip_prefix("0D"))
    {
        dec.parse::<i64>().ok()
    } else {
        digits.parse::<i64>().ok()
    };

    // Retorna Some(val) se parseou como i64, None se é BigInt grande.
    n.map(|v| v * sign)
}

// ── Pipeline JIT completo ───────────────────────────────────

/// Resultado da execução JIT — valor bruto + tipo canônico para display.
pub struct JitResult {
    /// Valor bruto retornado pela função JIT.
    /// Int: i64 SMI-taggeado. Float: f64 (reinterpretado). Text/Struct/Sum: ptr.
    pub raw: i64,
    /// Tipo canônico do entry point (para display).
    pub ty: Ty,
}

/// Compila e executa um `TypedModule` via Cranelift JIT.
///
/// Pipeline: criar JITBuilder → registrar símbolos FFI → declarar FFI →
/// lower_module → finalize_definitions → get_finalized_function →
/// transmutar → executar.
pub fn jit_eval(typed: &TypedModule) -> Result<JitResult, CodegenError> {
    let mut builder = cranelift_jit::JITBuilder::new(cranelift_module::default_libcall_names())
        .map_err(|e| CodegenError::Cranelift(format!("JITBuilder: {e}")))?;

    register_ffi_symbols(&mut builder);

    let mut module = cranelift_jit::JITModule::new(builder);

    let ffi_ids = declare_ffi_symbols(&mut module)?;

    // Declara __kata_entry e faz o lowering.
    let ret_ty = typed.entry.node.ty.clone();
    let (_metadata, _string_table) = lower_module(typed, &mut module, &ffi_ids)?;

    // Finaliza todas as definições — resolve relocations, compila machine code.
    module
        .finalize_definitions()
        .map_err(|e| CodegenError::Cranelift(format!("finalize_definitions: {e}")))?;

    // Obtém o ponteiro da função entry.
    let entry_id = module
        .get_name("__kata_entry")
        .ok_or_else(|| CodegenError::Cranelift("__kata_entry não encontrado".into()))?;
    let entry_fid = match entry_id {
        cranelift_module::FuncOrDataId::Func(fid) => fid,
        _ => return Err(CodegenError::Cranelift("__kata_entry não é função".into())),
    };
    let code = module.get_finalized_function(entry_fid);

    // Mantém o module vivo enquanto executamos — os ponteiros são válidos
    // apenas enquanto o JITModule existe.
    let result = match &ret_ty {
        Ty::Prim(PrimTy::Float) => {
            // Float: a função retorna f64, mas JIT usa calling convention
            // que pode retornar em registrador float. Transmutar para fn() -> f64.
            // SAFETY: `code` é ponteiro válido de Cranelift após finalize_definitions.
            // A assinatura da função foi construída com return type F64.
            let func: extern "C" fn() -> f64 = unsafe { std::mem::transmute(code) };
            let f = func();
            // Reinterpretar f64 como i64 para JitResult.raw.
            JitResult {
                raw: f.to_bits() as i64,
                ty: ret_ty,
            }
        }
        _ => {
            // Int, Text, Struct, Sum, Unit: retorna i64.
            // SAFETY: `code` é ponteiro válido de Cranelift após finalize_definitions.
            // A assinatura da função foi construída com return type I64.
            let func: extern "C" fn() -> i64 = unsafe { std::mem::transmute(code) };
            let val = func();
            JitResult {
                raw: val,
                ty: ret_ty,
            }
        }
    };

    // O module precisa sobreviver até aqui — dropping após execução.
    // Cranelift JIT mantém as páginas de código mapeadas enquanto o module vive.
    // Como `module` é dropped no fim deste escopo, o código já executou.
    std::mem::forget(module);

    Ok(result)
}
