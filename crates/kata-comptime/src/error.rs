//! Erros do comptime pass.

use kata_core::ty::Ty;
use kata_diagnostics::MietteSpan;
use thiserror::Error;

/// Erro do comptime pass — bug ou limitação do compilador, nunca do usuário.
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum ComptimeError {
    /// Expressão não é comptime-available (depende de valor runtime).
    #[error("expressão não é avaliável em compile-time: {reason}")]
    #[diagnostic(code = "comptime.not_available")]
    NotConsttime { reason: String },

    /// Expressão é impura (contém ActionCall, Fork, etc.).
    #[error("expressão é impura: {reason}")]
    #[diagnostic(code = "comptime.impure")]
    Impure { reason: String },

    /// Erro durante JIT execution.
    #[error("erro interno durante JIT em compile-time: {reason}")]
    #[diagnostic(
        code = "comptime.jit_failure",
        help = "abra uma issue com o código que causou este erro"
    )]
    JitError { reason: String },

    /// Tipo de resultado não suportado nesta fase.
    #[error("tipo não suportado em compile-time: {ty}")]
    #[diagnostic(code = "comptime.unsupported_type")]
    UnsupportedType { ty: Ty },

    /// Predicado de ascription refined falhou — erro de tipo do usuário,
    /// não bug do compilador. O valor não satisfaz o predicado declarado.
    #[error(
        "ascription refined falhou: o valor não satisfaz o predicado\n  help: o predicado retornou Boolean::False — verifique se o valor está dentro do domínio declarado"
    )]
    #[diagnostic(code = "type.refined_violation")]
    RefinedViolation {
        #[label("valor fora do domínio")]
        span: MietteSpan,
    },

    /// `constant` cujo value é uma lambda — Function não é serializável
    /// em compile-time (PRD §3.7). O usuário deve usar named function.
    #[error(
        "constant {name} — função (lambda) não é serializável em compile-time\n  help: use uma função nomeada em vez de `constant {name} := lambda ...`:\n        {name} :: {sig}\n        lambda ..."
    )]
    #[diagnostic(code = "constant.lambda_not_serializable")]
    ConstantLambda { name: String, sig: String },
}
