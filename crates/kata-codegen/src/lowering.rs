//! Lowering TAST → CLIF (Cranelift IR).
//!
//! Percorre a `TypedModule` (TAST) e gera código nativo via `FunctionBuilder`.
//! Sem IR intermediária própria — o lowering é direto TAST → CLIF.
//!
//! Block arguments nativos (Cranelift 0.133) — sem stack slots.
//! Variáveis locais (`let`) usam `Variable` + `def_var`/`use_var` do builder.

use std::collections::HashMap;

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ffi::FfiSymbol;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind, TypedModule};
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

/// Lower do `TypedModule` completo: cria a função `__kata_entry` e
/// retorna o `MetadataTable` sidecar + a string table.
pub fn lower_module(
    typed: &TypedModule,
    module: &mut cranelift_jit::JITModule,
    ffi_ids: &HashMap<String, cranelift_module::FuncId>,
) -> Result<(MetadataTable, StringTable), CodegenError> {
    let mut metadata = MetadataTable::new();
    let mut string_table = StringTable::new();

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
            metadata: &mut metadata,
            string_table: &mut string_table,
            var_map: HashMap::new(),
        };

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

/// Contexto de lowering — compartilhado entre as chamadas recursivas.
struct LowerCtx<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    module: &'a mut cranelift_jit::JITModule,
    ffi_refs: &'a HashMap<String, cranelift_codegen::ir::FuncRef>,
    #[allow(dead_code)]
    metadata: &'a mut MetadataTable,
    string_table: &'a mut StringTable,
    var_map: HashMap<String, cranelift_frontend::Variable>,
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

        // ── Closure: call FFI (call direto) ou call_indirect ──
        TypedExprKind::Closure {
            callee: _,
            args,
            ffi_symbol,
            captures: _,
            escapes: _,
        } => {
            let sym_name = ffi_symbol
                .as_ref()
                .ok_or_else(|| CodegenError::UnsupportedNode("Closure sem ffi_symbol (call_indirect não implementado ainda)".into()))?;

            let func_ref = ctx
                .ffi_refs
                .get(sym_name)
                .ok_or_else(|| CodegenError::FfiSymbolNotFound(sym_name.clone()))?;

            // Lowera os argumentos.
            let mut arg_values = Vec::with_capacity(args.len());
            for arg in args {
                let val = lower_expr(&arg.node, ctx)?;
                arg_values.push(val);
            }

            // Chama a função FFI.
            let call_inst = ctx.builder.ins().call(*func_ref, &arg_values);
            Ok(ctx.builder.inst_results(call_inst)[0])
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

        // ── Tuple: não suportado em Fio 1 ──
        TypedExprKind::Tuple { .. } => Err(CodegenError::UnsupportedNode(
            "tuple não suportado em Fio 1".into(),
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

        // ── Fio 2 Fase 8: Lambda e Match (lowering é Fase 9) ──
        TypedExprKind::Lambda { .. } => Err(CodegenError::UnsupportedNode(
            "lower_lambda: Fase 9 (não implementado ainda)".into(),
        )),
        TypedExprKind::Match { .. } => Err(CodegenError::UnsupportedNode(
            "lower_match: Fase 9 (não implementado ainda)".into(),
        )),
    }
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
