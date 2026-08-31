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
mod aot;
mod backend;
mod cache_key;
mod clause;
mod closure;
mod collections_hof;
mod collections_literal;
mod control_flow;
mod csp;
mod dict_set_lit;
mod escape_arena;
mod expr;
mod filter;
mod for_in;
mod function_def;
mod fused_stream;
mod jit;
mod map;
mod module;
mod pattern;
mod range_iter;
mod tail_call;
mod test_runner;
mod timer;
mod variant;

pub(crate) use backend::ModuleBackend;

pub use aot::aot_emit;
pub use jit::{
    JitResult, PrevFuncMap, ReplJitResult, jit_compile_tests, jit_eval, jit_eval_repl, leak_rt_ptr,
};
pub use module::CodegenError;
use module::StringTable;
pub use test_runner::TestWrapper;

use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{Block, GlobalValue, Value};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;

/// Kind de I/O handle — determina qual FFI de close chamar no epílogo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoHandleKind {
    File,
    Socket,
}
use std::collections::HashMap;

use crate::metadata::MetadataTable;
use kata_core::StructKey;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind};
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
            return Err(module::CodegenError::UnsupportedNode {
                node: format!(
                    "callee não-Ident em func_key_from_callee: {:?}",
                    callee.node.kind
                ),
            });
        }
    };
    match &callee.node.ty {
        Ty::Function(params, ret) => Ok((name, params.clone(), (**ret).clone())),
        _ => Err(module::CodegenError::UnsupportedNode {
            node: format!(
                "callee.ty não é Function em func_key_from_callee: {}",
                callee.node.ty
            ),
        }),
    }
}

/// Contexto de lowering — compartilhado entre as chamadas recursivas.
pub(crate) struct LowerCtx<'a, 'b> {
    pub builder: &'a mut FunctionBuilder<'b>,
    pub module: &'a mut dyn ModuleBackend,
    pub ffi_refs: &'a HashMap<String, cranelift_codegen::ir::FuncRef>,
    pub kata_refs: &'a HashMap<FuncKey, cranelift_codegen::ir::FuncRef>,
    /// FuncRefs das funções inner (wrapper/inner split). Default: HashMap vazio.
    /// Tail calls no inner resolvem para aqui (TCO); non-tail calls resolvem
    /// para `kata_refs` (wrapper, com cache/timer).
    pub kata_refs_inner: &'a HashMap<FuncKey, cranelift_codegen::ir::FuncRef>,
    /// FuncIds globais (module-level) para re-declaração em lambdas anônimos.
    pub ffi_ids: &'a HashMap<String, cranelift_module::FuncId>,
    pub kata_ids: &'a HashMap<FuncKey, cranelift_module::FuncId>,
    #[allow(dead_code)]
    pub metadata: &'a mut MetadataTable,
    pub string_table: &'a mut StringTable,
    pub bytes_table: &'a mut Vec<Vec<u8>>,
    pub var_map: HashMap<String, cranelift_frontend::Variable>,
    pub anon_counter: u32,
    /// Flag: se `true`, o último `lower_expr` emitiu um `return_call` (tail call).
    /// O caller NÃO deve emitir `return_` depois — a função já terminou.
    pub emitted_tail_call: bool,
    /// Flag: se `true`, o último `lower_expr` emitiu um terminador de block
    /// (break/continue/return — jump para break_block/continue_block/epilogue).
    /// O caller NÃO deve emitir jump/return_ depois — o block já está fechado.
    pub emitted_terminator: bool,
    /// Se `true`, tail calls estão desabilitados (entry point usa SystemV,
    /// não pode fazer return_call para funções Kata com CallConv::Tail).
    pub no_tail_calls: bool,
    /// Epílogo da Action atual — block que executa `return_`.
    /// `None` para funções puras e entry point. `Some(block)` dentro de Actions.
    /// `return` faz `jump epilogue_block(value)` com o valor de retorno.
    pub epilogue_block: Option<Block>,
    /// Handle da arena do fiber atual. Usado para alocar tuplas
    /// em computação local (não-tail-pos). `None` no entry point e funções puras.
    /// Substitui `local_arena` — agora a arena pertence ao fiber,
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
    /// I/O handles abertos que precisam de close no epílogo da action.
    /// Cada entrada é `(Variable, IoHandleKind)` — a Variable segura o
    /// handle (ponteiro para FileInner/SocketInner na arena) e o kind
    /// determina qual FFI de close chamar no epílogo.
    pub io_handle_vars: Vec<(cranelift_frontend::Variable, IoHandleKind)>,
    /// Catálogo de structs com alias_of/predicates — para resolver o
    /// Cranelift type correto de refined/alias de primitivos.
    pub struct_registry: &'a kata_core::StructRegistry,
    /// Mapa Ty → type_id para marshalling (spawn! to_bytes/from_bytes).
    /// Populado pelo driver via `build_and_register_type_table`.
    pub type_id_map: &'a HashMap<Ty, i64>,
    /// FuncId do broker IPC sintetizado (uma vez por compilação).
    /// `None` até a primeira `queue!(N)` cross-process solicitar a síntese.
    pub ipc_broker_fid: Option<cranelift_module::FuncId>,
    /// A2: Ponteiro do Runtime (`i64` como `Value`). Passado como primeiro
    /// argumento a todas as FFIs que precisam de estado de execução.
    /// `None` em funções puras (não chamam FFIs de runtime). Setado no
    /// entry point e test wrapper após `kata_rt_scheduler_init`.
    pub rt: Option<Value>,
    /// Se `true`, capturar a CLIF de cada função antes de `clear_context`.
    /// Propagado para `define_function_body` quando lambdas anônimas são compiladas.
    pub dump_ir: bool,
    /// Coletor da CLIF capturada — `(nome, clif_text)`.
    /// Ponteiro para o vec de `lower_module`.
    pub ir_dump: &'a mut Vec<(String, String)>,
}

/// Resolve o Cranelift type de um `Ty`, percorrendo a cadeia de `alias_of`
/// para refined/alias de primitivos. `Ty::Struct(Plain("PositiveFloat"))` → F64,
/// `Ty::Struct(Plain("Peso"))` → F64 (Peso → PositiveFloat → Float).
/// Para `Instance("NonZero", "Int")`, o tipo concreto já está no StructKey
/// — resolve direto sem consultar o registry.
pub(crate) fn resolve_clif_ty(
    ty: &Ty,
    struct_registry: &kata_core::StructRegistry,
) -> cranelift_codegen::ir::Type {
    if let Ty::Struct(key) = ty {
        // Instance carrega o tipo concreto — resolve direto.
        if let StructKey::Instance(_, concrete) = key {
            return match concrete.as_str() {
                "Int" => I64,
                "Float" => F64,
                "Text" | "Rational" => I64,
                _ => {
                    // Alias de outro struct — percorrer a cadeia.
                    let mut current = concrete.clone();
                    while let Some(info) = struct_registry.get(&current) {
                        if let Some(base) = &info.alias_of {
                            match base.as_str() {
                                "Int" => return I64,
                                "Float" => return F64,
                                "Text" | "Rational" => return I64,
                                _ => {
                                    current = base.clone();
                                    continue;
                                }
                            }
                        }
                        break;
                    }
                    I64 // fallback: structs são ponteiros (I64)
                }
            };
        }
        // Plain: percorre cadeia de alias_of no registry.
        let mut current = key.name().to_string();
        while let Some(info) = struct_registry.get(&current) {
            if let Some(base) = &info.alias_of {
                match base.as_str() {
                    "Int" => return I64,
                    "Float" => return F64,
                    "Text" | "Rational" => return I64,
                    _ => {
                        current = base.clone();
                        continue;
                    }
                }
            }
            break;
        }
    }
    crate::ffi_sigs::ty_to_clif(ty)
}

/// Verifica se um `Ty` é Rational (direto ou via refined/alias).
/// Refineds como `data (Rational, ...) as RatUmOuDois` têm `alias_of = "Rational"`
/// no StructRegistry.
pub(crate) fn is_rational_based(ty: &Ty, struct_registry: &kata_core::StructRegistry) -> bool {
    match ty {
        Ty::Prim(PrimTy::Rational) => true,
        Ty::Struct(key) => {
            let mut current = key.name().to_string();
            while let Some(info) = struct_registry.get(&current) {
                if let Some(base) = &info.alias_of {
                    match base.as_str() {
                        "Rational" => return true,
                        _ => {
                            current = base.clone();
                            continue;
                        }
                    }
                }
                break;
            }
            false
        }
        _ => false,
    }
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

    /// Adiciona bytes crus à bytes table e retorna o GlobalValue.
    /// Os bytes são definidos como data symbol no module (sem null terminator).
    pub(crate) fn add_bytes(&mut self, bytes: &[u8]) -> GlobalValue {
        let idx = self.bytes_table.len();
        self.bytes_table.push(bytes.to_vec());
        let sym = format!("__kata_bytes_{idx}");
        let did = self
            .module
            .declare_data(&sym, cranelift_module::Linkage::Local, false, false)
            .expect("declare_data falhou para bytes literal");
        self.module.declare_data_in_func(did, self.builder.func)
    }

    /// Gera um nome fresh para lambda anônimo.
    pub(crate) fn fresh_anon_name(&mut self) -> String {
        let name = format!("__anon_{}", self.anon_counter);
        self.anon_counter += 1;
        name
    }
}
