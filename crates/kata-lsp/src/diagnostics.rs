//! Conversão de FrontendError/MiddleError → LSP Diagnostic.
//!
//! Todos os erros do front-end são ERROR (não há warnings no front-end atual).
//! O `Span` (byte offsets) é extraído via `miette::Diagnostic::labels()`,
//! que retorna `LabeledSpan` com `SourceSpan { offset, len }`.
//!
//! `ResolveError` não implementa `miette::Diagnostic` — tratado separadamente
//! sem span (usa position 0:0).

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

use crate::analysis::FrontendError;
use crate::unicode::byte_offset_to_lsp_position;

/// Converte um erro do front-end em `Diagnostic` do LSP.
pub(crate) fn to_diagnostic(error: &FrontendError, text: &str) -> Diagnostic {
    let (code, message, span) = match error {
        FrontendError::Lex(e) | FrontendError::Parse(e) => {
            let code = miette::Diagnostic::code(e).map(|c| c.to_string());
            let message = e.to_string();
            let span = extract_span(e);
            (code, message, span)
        }
        FrontendError::Infer(e) => {
            let code = miette::Diagnostic::code(e).map(|c| c.to_string());
            let message = e.to_string();
            let span = extract_span(e);
            (code, message, span)
        }
        FrontendError::Resolve(errors) => {
            // ResolveError implementa miette::Diagnostic (códigos resolve.*).
            // Sem #[label] — span 0:0 (sem info de span).
            if let Some(first) = errors.first() {
                let code = miette::Diagnostic::code(first).map(|c| c.to_string());
                let message = first.to_string();
                (code, message, None)
            } else {
                (None, "erro de resolução".to_string(), None)
            }
        }
    };

    let range = span_to_range(text, span);

    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: code.map(tower_lsp::lsp_types::NumberOrString::String),
        source: Some("kata".to_string()),
        message,
        ..Default::default()
    }
}

/// Converte uma lista de erros do front-end em uma lista de Diagnostics.
pub(crate) fn to_diagnostics(errors: &[FrontendError], text: &str) -> Vec<Diagnostic> {
    errors.iter().map(|e| to_diagnostic(e, text)).collect()
}

/// Extrai o byte offset e len do primeiro `#[label]` de um `miette::Diagnostic`.
fn extract_span(diag: &dyn miette::Diagnostic) -> Option<(usize, usize)> {
    let labels = diag.labels()?;
    let first = labels.into_iter().next()?;
    let span = first.inner();
    Some((span.offset(), span.len()))
}

/// Converte um `Option<(byte_offset, len)>` em `Range` LSP.
/// Se None, usa Position 0:0 com range vazio.
fn span_to_range(text: &str, span: Option<(usize, usize)>) -> Range {
    match span {
        Some((offset, len)) => {
            let start = byte_offset_to_lsp_position(text, offset);
            let end = byte_offset_to_lsp_position(text, offset + len);
            Range { start, end }
        }
        None => Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        },
    }
}
