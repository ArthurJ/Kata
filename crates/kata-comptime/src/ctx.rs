//! Contexto partilhado do comptime pass — dados imutáveis do módulo
//! e resultado bruto da execução JIT.

use kata_core::EnumRegistry;
use kata_core::StructRegistry;
use kata_core::dispatch::DispatchTable;
use kata_core::ty::{Ty, TypeEnv};
use kata_inference::{TypedAction, TypedFunction};

/// Dados imutáveis do módulo necessários para constness e JIT execution.
///
/// Referências aos campos individuais de `TypedModule` — Rust permite borrow
/// de campos diferentes do mesmo struct simultaneamente (partial borrow),
/// evitando o conflito entre `&mut current.pre_entry`/`&mut current.entry`
/// e `&current.dispatch_table` etc.
pub(crate) struct ModuleCtx<'a> {
    pub dispatch_table: &'a DispatchTable,
    pub type_env: &'a TypeEnv,
    pub functions: &'a [TypedFunction],
    pub actions: &'a [TypedAction],
    pub struct_registry: &'a StructRegistry,
    pub enum_registry: &'a EnumRegistry,
    /// Ponteiro bruto do Runtime compartilhado entre comptime e execução
    /// principal. Permite que `set_recursion_limit` executado em comptime
    /// persista o `depth_limit` para o Runtime da execução principal.
    pub rt_ptr: i64,
}

/// Resultado da execução comptime — valor bruto + tipo.
pub(crate) struct ComptimeResult {
    pub raw: i64,
    pub ty: Ty,
}
