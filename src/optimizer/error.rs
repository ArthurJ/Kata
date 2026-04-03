use crate::parser::ast::Span;

#[derive(Debug, Clone)]
pub struct OptimizerError {
    pub message: crate::errors::KataError,
    pub _span: Span,
}

impl OptimizerError {
    pub fn new(message: crate::errors::KataError, span: Span) -> Self {
        Self { message, _span: span }
    }
}
