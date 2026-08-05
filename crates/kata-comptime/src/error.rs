//! Erros do comptime pass.

/// Erro do comptime pass.
#[derive(Debug)]
pub enum ComptimeError {
    /// Expressão não é comptime-available (depende de valor runtime).
    NotConsttime { reason: String },
    /// Expressão é impura (contém ActionCall, Fork, etc.).
    Impure { reason: String },
    /// Erro durante JIT execution.
    JitError { reason: String },
    /// Tipo de resultado não suportado nesta fase.
    UnsupportedType { ty: kata_core::ty::Ty },
}

impl std::fmt::Display for ComptimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComptimeError::NotConsttime { reason } => {
                write!(f, "não é comptime-available: {reason}")
            }
            ComptimeError::Impure { reason } => {
                write!(f, "expressão impura: {reason}")
            }
            ComptimeError::JitError { reason } => {
                write!(f, "erro de JIT: {reason}")
            }
            ComptimeError::UnsupportedType { ty } => {
                write!(f, "tipo não suportado em comptime (Fase 1): {ty}")
            }
        }
    }
}

impl std::error::Error for ComptimeError {}