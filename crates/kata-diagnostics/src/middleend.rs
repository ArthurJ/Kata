//! Erros do middleend (resolution + inference).
//!
//! Carregam [`Span`] apontando para o código-fonte do usuário.

use crate::frontend::MietteSpan;
use thiserror::Error;

/// Erro do middleend (resolução, tipos, dispatch).
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum MiddleError {
    #[error("nome `{name}` não está no escopo")]
    #[diagnostic(code = "type.unbound_name")]
    UnboundName {
        name: String,
        #[label]
        span: MietteSpan,
    },

    #[error("tipo incompatível: esperado `{expected}`, encontrado `{found}`")]
    #[diagnostic(code = "type.mismatch")]
    TypeMismatch {
        expected: String,
        found: String,
        #[label]
        span: MietteSpan,
    },

    #[error("dispatch ambíguo: múltiplas sobrecargas compatíveis para `{name}`")]
    #[diagnostic(code = "type.ambiguous_dispatch")]
    AmbiguousDispatch {
        name: String,
        #[label]
        span: MietteSpan,
    },

    #[error("nenhuma sobrecarga compatível para `{name}` com os argumentos fornecidos")]
    #[diagnostic(code = "type.no_overload")]
    NoOverload {
        name: String,
        #[label]
        span: MietteSpan,
    },

    #[error("tipo `{name}` já declarado")]
    #[diagnostic(code = "type.duplicate_decl")]
    DuplicateDecl {
        name: String,
        #[label]
        span: MietteSpan,
    },

    #[error("número incorreto de argumentos: esperado {expected}, encontrado {found}")]
    #[diagnostic(code = "type.arity_mismatch")]
    ArityMismatch {
        expected: usize,
        found: usize,
        #[label]
        span: MietteSpan,
    },

    #[error("símbolo FFI desconhecido: `{name}`")]
    #[diagnostic(code = "type.unknown_ffi")]
    UnknownFfi {
        name: String,
        #[label]
        span: MietteSpan,
    },
}
