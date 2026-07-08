//! `Span` — localização no código-fonte.
//!
//! Carrega offset absoluto, linha, coluna e comprimento.
//! Erros do usuário carregam `Span` para apontar ao código-fonte.
//! Erros internos do compilador não carregam `Span` (I6 no manual).

use std::fmt;

/// Offset absoluto (bytes) no arquivo, linha (1-indexed), coluna (1-indexed),
/// e comprimento em bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Span {
    /// Offset absoluto em bytes a partir do início do arquivo.
    pub offset: usize,
    /// Linha (1-indexed).
    pub line: usize,
    /// Coluna (1-indexed, em bytes).
    pub col: usize,
    /// Comprimento em bytes.
    pub len: usize,
}

impl Span {
    pub fn new(offset: usize, line: usize, col: usize, len: usize) -> Self {
        Span {
            offset,
            line,
            col,
            len,
        }
    }

    /// Span de comprimento zero — útil para tokens sintéticos ou quando
    /// o span exato não está disponível.
    pub fn zero() -> Self {
        Span {
            offset: 0,
            line: 1,
            col: 1,
            len: 0,
        }
    }

    /// Cria um span sintético (sem localização real).
    /// Para nós TAST que não têm correspondência direta no código-fonte.
    pub fn synthetic() -> Self {
        Span {
            offset: 0,
            line: 0,
            col: 0,
            len: 0,
        }
    }

    /// Verifica se este span é sintético (não aponta para código real).
    pub fn is_synthetic(&self) -> bool {
        self.line == 0
    }

    /// Span que cobre dois spans (do início do menor ao fim do maior).
    pub fn cover(&self, other: Span) -> Span {
        let start = self.offset.min(other.offset);
        let end = (self.offset + self.len).max(other.offset + other.len);
        let line = if self.offset <= other.offset {
            self.line
        } else {
            other.line
        };
        let col = if self.offset <= other.offset {
            self.col
        } else {
            other.col
        };
        Span {
            offset: start,
            line,
            col,
            len: end - start,
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}@{}+{}", self.line, self.col, self.offset, self.len)
    }
}

/// Anexa `Span` a qualquer nó da AST/TAST.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Spanned { node, span }
    }

    pub fn map<U, F>(self, f: F) -> Spanned<U>
    where
        F: FnOnce(T) -> U,
    {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }

    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            node: &self.node,
            span: self.span,
        }
    }
}
