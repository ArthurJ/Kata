//! Pass 2: inference.
//!
//! Type-check dos corpos, inferência de tipos, dispatch por dominância.
//! Produz o `TypedModule` (TAST) com `ty`, `tail_pos: bool`
//! em cada nó.
//!
//! Entry point: [`infer_module`] — consome `Module` (AST) + `ResolvedModule`
//! e produz `TypedModule`.

pub mod desugar;
pub(crate) mod desugar_holes;
pub(crate) mod infer;
pub(crate) mod patterns;
pub(crate) mod redundancy;
pub(crate) mod typed;
pub(crate) mod typed_module;
pub(crate) mod typed_pattern;

pub use infer::generics::{Substitutions, apply_subs, unify};
pub use infer::infer_module;
pub use typed::{
    CacheSpec, CaptureInfo, ChannelKind, FusedStage, TypedAction, TypedExpr, TypedExprKind,
    TypedFunction, TypedLogSpec, TypedModule, TypedReadMode, TypedSelectArm, TypedTestSpec,
};
pub use typed_pattern::{
    TypedGuardClause, TypedLambdaClause, TypedMatchArm, TypedPattern, TypedWithBinding,
};
