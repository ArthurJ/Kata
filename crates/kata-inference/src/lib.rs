//! Pass 2: inference.
//!
//! Type-check dos corpos, inferência de tipos, dispatch por dominância.
//! Produz o `TypedModule` (TAST) com `ty`, `tail_pos: bool`
//! em cada nó.
//!
//! Entry point: [`infer_module`] — consome `Module` (AST) + `ResolvedModule`
//! e produz `TypedModule`.

pub mod desugar;
pub mod desugar_directives;
pub(crate) mod desugar_holes;
pub(crate) mod guard_completeness;
pub(crate) mod infer;
pub(crate) mod maranget;
pub(crate) mod patterns;
pub(crate) mod redundancy;
pub(crate) mod typed;
pub(crate) mod typed_module;
pub(crate) mod typed_pattern;
pub(crate) mod z3_translate;

pub use infer::generics::{Substitutions, apply_subs, unify};
pub use infer::infer_module;
pub use infer::wrap_entry_with_show;
pub use typed::{
    CacheSpec, CacheStrategy, CaptureInfo, ChannelKind, FusedStage, TypedAction, TypedExpr,
    TypedExprKind, TypedFunction, TypedModule, TypedReadMode, TypedSelectArm, TypedTestSpec,
};
pub use typed_module::TimerSpec;
pub use typed_pattern::{
    TypedGuardClause, TypedLambdaClause, TypedMatchArm, TypedPattern, TypedWithBinding,
};
