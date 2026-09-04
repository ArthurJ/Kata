//! Catálogo de erros de dispatch.
//!
//! [`DispatchError`] é um enum puro de dados — não contém lógica,
//! apenas variantes carregando informações de diagnóstico.

/// Erro de dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    FunctionNotFound {
        name: String,
        arg_count: usize,
    },
    TypeMismatch {
        name: String,
        expected: String,
        found: String,
    },
    AmbiguousDispatch {
        name: String,
        arg_count: usize,
    },
}
