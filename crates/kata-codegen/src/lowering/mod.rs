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

mod clause;
mod expr;
mod jit;
mod _match;
mod module;
mod pattern;

pub use jit::{jit_eval, JitResult};
pub use module::CodegenError;
use module::StringTable;

use cranelift_codegen::ir::GlobalValue;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use std::collections::HashMap;

use crate::metadata::MetadataTable;

/// Contexto de lowering — compartilhado entre as chamadas recursivas.
pub(crate) struct LowerCtx<'a, 'b> {
    pub builder: &'a mut FunctionBuilder<'b>,
    pub module: &'a mut cranelift_jit::JITModule,
    pub ffi_refs: &'a HashMap<String, cranelift_codegen::ir::FuncRef>,
    pub kata_refs: &'a HashMap<String, cranelift_codegen::ir::FuncRef>,
    /// FuncIds globais (module-level) para re-declaração em lambdas anônimos.
    pub ffi_ids: &'a HashMap<String, cranelift_module::FuncId>,
    pub kata_ids: &'a HashMap<String, cranelift_module::FuncId>,
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