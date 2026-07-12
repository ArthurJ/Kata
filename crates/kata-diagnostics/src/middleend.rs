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
        #[label("tipos incompatíveis")]
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

    #[error("match não-exaustivo: nem todas as variantes estão cobertas")]
    #[diagnostic(code = "type.non_exhaustive_match")]
    NonExhaustiveMatch {
        /// Variantes que faltam (ex: "False").
        missing: Vec<String>,
        #[label]
        span: MietteSpan,
    },

    #[error("guard sem `otherwise` em tipo infinito")]
    #[diagnostic(code = "type.missing_otherwise")]
    MissingOtherwise {
        #[label]
        span: MietteSpan,
    },

    #[error("cláusula redundante: sombreada por cláusula anterior")]
    #[diagnostic(code = "type.redundant_clause")]
    RedundantClause {
        #[label]
        span: MietteSpan,
    },

    #[error(
        "não foi possível inferir os tipos dos parâmetros do lambda — forneça uma anotação de tipo (ex: ::(Int -> Int))"
    )]
    #[diagnostic(code = "type.lambda_inference_fail")]
    LambdaInferenceFail {
        #[label("lambda sem tipo inferível")]
        span: MietteSpan,
    },

    #[error("ação `{action}` é recursiva: ciclo detectado ({cycle})")]
    #[diagnostic(code = "action.recursive")]
    RecursiveAction {
        action: String,
        cycle: String,
        #[label]
        span: MietteSpan,
    },
}
