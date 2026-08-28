//! `kata-interp` — Interpretador tree-walking sobre TAST.
//!
//! Terceiro backend do compilador Kata. Consome o `TypedModule` pós-`optimize()`
//! sem passar por codegen/Cranelift. Reusa o runtime `kata-rt` diretamente.

use std::sync::Arc;

use kata_core::ty::Ty;
use kata_inference::TypedModule;

pub use env::Env;
pub use eval::{InterpCtx, InterpError, eval};

mod csp;
mod env;
mod eval;
mod ffi_dispatch;
mod show;
mod value;

/// Resultado da interpretação — valor bruto + tipo para display.
pub struct InterpResult {
    pub raw: i64,
    pub ty: Ty,
}

/// Interpreta um `TypedModule` com o runtime dado.
///
/// Cria o contexto, avalia o entry point, e retorna o resultado.
/// O caller é responsável pelo lifecycle do Runtime.
pub fn interpret(module: TypedModule, rt_ptr: i64) -> Result<InterpResult, InterpError> {
    let ty = module.entry.node.ty.clone();
    let mut ctx = eval::InterpCtx::new(module, rt_ptr);
    let raw = ctx.eval_entry()?;
    Ok(InterpResult { raw, ty })
}

/// Interpreta um `TypedModule` com enum registry (para show de variants).
///
/// Cria o contexto com `new_with_registry`, avalia o entry point, e retorna
/// o resultado. O enum registry é usado por `show_sum` para mapear tag → nome
/// de variante.
pub fn interpret_with_registry(
    module: TypedModule,
    rt_ptr: i64,
    enum_registry: kata_core::EnumRegistry,
) -> Result<InterpResult, InterpError> {
    let ty = module.entry.node.ty.clone();
    let mut ctx = eval::InterpCtx::new_with_registry(module, rt_ptr, Arc::new(enum_registry));
    let raw = ctx.eval_entry()?;
    Ok(InterpResult { raw, ty })
}

/// Interpreta um `TypedModule` reusando um `Env` persistente (para REPL).
///
/// O `Env` deve conter bindings `let` acumulados de linhas anteriores.
/// O interpretador avalia constants, pre_entry e entry point usando
/// este env em vez de criar um novo.
pub fn interpret_with_env(
    module: TypedModule,
    rt_ptr: i64,
    env: &mut Env,
) -> Result<InterpResult, InterpError> {
    let ty = module.entry.node.ty.clone();
    let mut ctx = eval::InterpCtx::new(module, rt_ptr);
    let raw = ctx.eval_entry_with_env(env)?;
    Ok(InterpResult { raw, ty })
}