//! Erros do frontend (lexer + parser).
//!
//! Carregam [`Span`] apontando para o código-fonte do usuário.
//! Reportados com miette colorido.
//!
//! O `#[label]` do miette exige um tipo que implemente `Into<SourceSpan>`.
//! Como não podemos implementar `From<Span> for SourceSpan` (orphan rule —
//! nenhum dos dois está neste crate), usamos um newtype wrapper `MietteSpan`
//! que envolve `Span` e implementa a conversão.

use kata_ast::Span;
use thiserror::Error;

/// Wrapper de `Span` que implementa `Into<miette::SourceSpan>`.
/// Usado nos campos `#[label]` dos erros.
#[derive(Debug, Clone, Copy)]
pub struct MietteSpan(pub Span);

impl From<MietteSpan> for miette::SourceSpan {
    fn from(s: MietteSpan) -> Self {
        miette::SourceSpan::new(s.0.offset.into(), s.0.len)
    }
}

impl From<Span> for MietteSpan {
    fn from(s: Span) -> Self {
        MietteSpan(s)
    }
}

/// Erro do frontend (léxico ou sintático).
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum FrontendError {
    #[error("token inesperado: esperado `{expected}`, encontrado `{found}`")]
    #[diagnostic(code = "parse.unexpected_token")]
    UnexpectedToken {
        expected: String,
        found: String,
        #[label("token inesperado")]
        span: MietteSpan,
    },

    #[error("caractere inválido: `{char}`")]
    #[diagnostic(code = "lex.invalid_char")]
    InvalidChar {
        char: String,
        #[label("caractere inválido")]
        span: MietteSpan,
    },

    #[error("string não terminada")]
    #[diagnostic(code = "lex.unterminated_string")]
    UnterminatedString {
        #[label("string não terminada")]
        span: MietteSpan,
    },

    #[error("número inválido: `{text}`")]
    #[diagnostic(code = "lex.invalid_number")]
    InvalidNumber {
        text: String,
        #[label("número inválido")]
        span: MietteSpan,
    },

    #[error("esperado fim de arquivo, encontrado `{found}`")]
    #[diagnostic(code = "parse.trailing_tokens")]
    TrailingTokens {
        found: String,
        #[label("tokens após fim do programa")]
        span: MietteSpan,
    },

    #[error("esperado expressão, encontrado `{found}`")]
    #[diagnostic(code = "parse.expected_expr")]
    ExpectedExpr {
        found: String,
        #[label("esperado expressão")]
        span: MietteSpan,
    },

    #[error("indentação inconsistente: esperado {expected} espaços, encontrado {found}")]
    #[diagnostic(code = "lex.inconsistent_indent")]
    InconsistentIndent {
        expected: usize,
        found: usize,
        #[label("indentação inconsistente")]
        span: MietteSpan,
    },

    #[error("nome `{name}` deve ser {expected_casing}, mas está em {found_casing}")]
    #[diagnostic(code = "parse.invalid_casing")]
    InvalidCasing {
        name: String,
        expected_casing: String,
        found_casing: String,
        #[label("casing inválido")]
        span: MietteSpan,
    },
}
