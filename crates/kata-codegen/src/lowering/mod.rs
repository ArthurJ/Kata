//! Lowering TAST → CLIF (Cranelift IR) — diretório de submódulos.
//!
//! O lowering é direto TAST → CLIF, sem IR intermediária própria.
//! Block arguments nativos (Cranelift 0.133) — sem stack slots.
//!
//! Submódulos:
//! - [`mod@module`] — `lower_module`, declaração/definição de funções Kata
//! - [`mod@expr`] — `lower_expr` (dispatch central TAST → CLIF)
//! - [`mod@clause`] — cláusulas lambda: guards, branch chain, with bindings
//! - [`super::pattern`] — teste de patterns (clause + match)
//! - [`mod@match`] — `lower_match` (pattern matching)
//! - [`mod@jit`] — pipeline JIT completo (`jit_eval`)
//!
//! Tipos compartilhados (`LowerCtx`, `CodegenError`, `StringTable`) vivem
//! aqui no `mod.rs` e são importados pelos submódulos via `super::`.

mod _match;
mod action_call;
mod action_def;
mod clause;
mod closure;
mod control_flow;
mod expr;
mod function_def;
mod jit;
mod module;
mod pattern;

pub use jit::{JitResult, jit_eval};
pub use module::CodegenError;
use module::StringTable;

use cranelift_codegen::ir::{Block, GlobalValue, Value};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use std::collections::HashMap;

use crate::metadata::MetadataTable;
use kata_core::ty::Ty;
use kata_inference::{CaptureInfo, TypedExpr, TypedExprKind};
pub(crate) use module::FuncKey;

/// Extrai a chave composta `(name, param_types, ret_ty)` de um callee tipado.
///
/// Usado nos sites de lookup em `kata_refs`/`kata_ids` para funções Kata puras.
/// O `callee` deve ter `ty: Ty::Function(params, ret)` e `kind: Ident { name }`.
pub(crate) fn func_key_from_callee(
    callee: &kata_ast::Spanned<TypedExpr>,
) -> Result<FuncKey, module::CodegenError> {
    let name = match &callee.node.kind {
        TypedExprKind::Ident { name } => name.clone(),
        _ => {
            return Err(module::CodegenError::UnsupportedNode(format!(
                "callee não-Ident em func_key_from_callee: {:?}",
                callee.node.kind
            )));
        }
    };
    match &callee.node.ty {
        Ty::Function(params, ret) => Ok((name, params.clone(), (**ret).clone())),
        _ => Err(module::CodegenError::UnsupportedNode(format!(
            "callee.ty não é Function em func_key_from_callee: {:?}",
            callee.node.ty
        ))),
    }
}

/// Contexto de lowering — compartilhado entre as chamadas recursivas.
pub(crate) struct LowerCtx<'a, 'b> {
    pub builder: &'a mut FunctionBuilder<'b>,
    pub module: &'a mut cranelift_jit::JITModule,
    pub ffi_refs: &'a HashMap<String, cranelift_codegen::ir::FuncRef>,
    pub kata_refs: &'a HashMap<FuncKey, cranelift_codegen::ir::FuncRef>,
    /// FuncIds globais (module-level) para re-declaração em lambdas anônimos.
    pub ffi_ids: &'a HashMap<String, cranelift_module::FuncId>,
    pub kata_ids: &'a HashMap<FuncKey, cranelift_module::FuncId>,
    #[allow(dead_code)]
    pub metadata: &'a mut MetadataTable,
    pub string_table: &'a mut StringTable,
    pub var_map: HashMap<String, cranelift_frontend::Variable>,
    pub anon_counter: u32,
    /// Flag: se `true`, o último `lower_expr` emitiu um `return_call` (tail call).
    /// O caller NÃO deve emitir `return_` depois — a função já terminou.
    pub emitted_tail_call: bool,
    /// Se `true`, tail calls estão desabilitados (entry point usa SystemV,
    /// não pode fazer return_call para funções Kata com CallConv::Tail).
    pub no_tail_calls: bool,
    /// Epílogo da Action atual — block que executa `return_`.
    /// `None` para funções puras e entry point. `Some(block)` dentro de Actions.
    /// `return` faz `jump epilogue_block(value)` com o valor de retorno.
    pub epilogue_block: Option<Block>,
    /// Handle da arena do fiber atual. Usado para alocar tuplas
    /// em computação local (não-tail-pos). `None` no entry point e funções puras.
    /// Substitui `local_arena` da Fase 3 — agora a arena pertence ao fiber,
    /// não é criada/destruída no prólogo/epílogo da Action.
    pub fiber_arena: Option<Value>,
    /// Handle da arena do caller. Usado para alocar tuplas que sobrevivem
    /// à destruição da arena do fiber (valores em tail_pos). `None` fora de
    /// Actions e entry point.
    pub caller_arena: Option<Value>,
    /// Se `true`, ActionCalls definidas pelo usuário emitem `spawn+run` (entry point).
    /// Se `false`, ActionCalls definidas pelo usuário emitem `call` direto (dentro de Action).
    pub scheduler_mode: bool,
    /// Block de saída do loop atual — `break` faz `jump` para este block.
    /// `None` fora de um loop.
    pub loop_break_block: Option<Block>,
    /// Block de continuação do loop atual — `continue` faz `jump` para este block.
    /// `None` fora de um loop.
    pub loop_continue_block: Option<Block>,
    /// Captures de closures let-bound: mapeia nome → lista de captures.
    /// Populado quando `let f := lambda...` é lowerado. Usado no call site
    /// para alocar o CaptureBox e passar `box_ptr` como primeiro arg.
    pub closure_captures: HashMap<String, Vec<CaptureInfo>>,
}

impl<'a, 'b> LowerCtx<'a, 'b> {
    /// Declara uma nova variável no builder e mapeia o nome.
    pub(crate) fn new_var(
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
    pub(crate) fn add_string(&mut self, text: &str) -> GlobalValue {
        let idx = self.string_table.len();
        self.string_table.push(text.to_string());
        let sym = format!("__kata_str_{idx}");
        let did = self
            .module
            .declare_data(&sym, cranelift_module::Linkage::Local, false, false)
            .expect("declare_data falhou para string literal");
        self.module.declare_data_in_func(did, self.builder.func)
    }

    /// Gera um nome fresh para lambda anônimo.
    pub(crate) fn fresh_anon_name(&mut self) -> String {
        let name = format!("__anon_{}", self.anon_counter);
        self.anon_counter += 1;
        name
    }
}
