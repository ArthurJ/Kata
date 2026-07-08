//! Pass 2: inference.
//!
//! Type-check dos corpos, inferência de tipos, dispatch por dominância.
//! Produz o `TypedModule` (TAST) com `ty`, `tail_pos: bool`, `effect: Effect`
//! em cada nó.
//!
//! Entry point: [`infer_module`] — consome `Module` (AST) + `ResolvedModule`
//! e produz `TypedModule`.

pub mod desugar;
pub mod infer;
pub mod typed;

pub use infer::{InferResult, infer_module};
pub use typed::{Effect, TypedExpr, TypedExprKind, TypedModule};
