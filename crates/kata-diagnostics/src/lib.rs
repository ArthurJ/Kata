//! Erros estruturados do compilador — 1 crate com 3 submódulos.
//!
//! - [`frontend`] — erros léxicos e sintáticos (carregam Span, reportados com miette)
//! - [`middleend`] — erros de tipos e resolução (carregam Span)
//!
//! Códigos namespaced por domínio (`type.mismatch`, `parse.unexpected_token`),
//! sem códigos numéricos.

pub(crate) mod frontend;
pub(crate) mod middleend;

pub use frontend::{FrontendError, MietteSpan};
pub use middleend::MiddleError;
