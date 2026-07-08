//! Erros internos do backend (codegen/emit).
//!
//! Não carregam [`Span`] — não há código do usuário para apontar (I6).
//! Usam `expect()` com mensagens descritivas em vez de `unwrap()`.

use thiserror::Error;

/// Erro interno do backend (bug do compilador, não do usuário).
#[derive(Debug, Clone, Error)]
pub enum BackendError {
    #[error("codegen: tipo não suportado no lowering: {ty}")]
    UnsupportedType { ty: String },

    #[error("codegen: símbolo FFI não registrado: {symbol}")]
    UnregisteredFfi { symbol: String },

    #[error("codegen: função não definida: {name}")]
    UndefinedFunction { name: String },

    #[error("codegen: falha no Cranelift JIT: {reason}")]
    JitFailure { reason: String },

    #[error("codegen: assinatura não encontrada para: {name}")]
    MissingSignature { name: String },

    #[error("codegen: MetadataTable inconsistente: {detail}")]
    MetadataInconsistency { detail: String },
}
