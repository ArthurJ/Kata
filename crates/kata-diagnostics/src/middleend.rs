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
        /// Contexto sobre por que a inferência falhou (None quando nenhum
        /// mecanismo era aplicável — body não é Apply, callee desconhecido, etc).
        #[help]
        detail: Option<String>,
    },

    #[error("ação `{action}` é recursiva: ciclo detectado ({cycle})")]
    #[diagnostic(code = "action.recursive")]
    RecursiveAction {
        action: String,
        cycle: String,
        #[label]
        span: MietteSpan,
    },

    #[error("struct `{struct_name}` não tem campo `{field_name}`")]
    #[diagnostic(code = "type.unknown_field")]
    UnknownField {
        struct_name: String,
        field_name: String,
        #[label]
        span: MietteSpan,
    },

    #[error("índice {index} fora dos limites da tupla de {len} elementos")]
    #[diagnostic(code = "type.index_out_of_bounds")]
    IndexOutOfBounds {
        index: i64,
        len: usize,
        #[label]
        span: MietteSpan,
    },

    #[error("tipo `{ty}` não suporta acesso por `.` (esperado struct ou tupla)")]
    #[diagnostic(code = "type.not_indexable")]
    NotIndexable {
        ty: String,
        #[label]
        span: MietteSpan,
    },

    #[error("tupla não tem campos nomeados — use `.N` para indexar")]
    #[diagnostic(code = "type.field_access_on_tuple")]
    FieldAccessOnTuple {
        #[label]
        span: MietteSpan,
    },

    #[error("struct não é indexável — use `.nome` para acessar campos")]
    #[diagnostic(code = "type.index_access_on_struct")]
    IndexAccessOnStruct {
        #[label]
        span: MietteSpan,
    },

    #[error("canais não podem ser retornados de Actions — `{ty}` contém `Sender`, `Receiver`, ou `ReceiverFactory`")]
    #[diagnostic(code = "type.channel_in_return")]
    ChannelInReturn {
        ty: String,
        #[label]
        span: MietteSpan,
    },
}
