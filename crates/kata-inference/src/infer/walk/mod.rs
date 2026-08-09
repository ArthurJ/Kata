//! Walkers da TAST — percorrem sub-expressões de um `TypedExpr`.
//!
//! `for_each_subexpr` (imutável) e `for_each_subexpr_mut` (mutável) descem
//! recursivamente nos filhos de cada nó, chamando `f` em pré-ordem.
//! Se `f` retorna `false`, a descida nos filhos desse nó é abortada.
//!
//! Extraído de `infer/expr.rs` para reutilização em captures, tree shaking,
//! monomorph, comptime, etc.

mod immut;
mod mut_vis;

pub(crate) use immut::for_each_subexpr;
pub(crate) use mut_vis::for_each_subexpr_mut;
