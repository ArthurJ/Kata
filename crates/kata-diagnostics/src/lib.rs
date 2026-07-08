//! Erros estruturados do compilador — 1 crate com 3 submódulos.
//!
//! - [`frontend`] — erros léxicos e sintáticos (carregam Span, reportados com miette)
//! - [`middleend`] — erros de tipos e resolução (carregam Span)
//! - [`backend`] — erros internos de codegen (não carregam Span — bugs nossos)
//!
//! Códigos namespaced por domínio (`type.mismatch`, `parse.unexpected_token`),
//! sem códigos numéricos.

pub mod backend;
pub mod frontend;
pub mod middleend;
