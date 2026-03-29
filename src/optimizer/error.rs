use crate::parser::ast::Span;

#[derive(Debug, Clone)]
pub struct OptimizerError {
    pub message: String,
    pub span: Span,
}

impl OptimizerError {
    pub fn new(message: String, span: Span) -> Self {
        Self { message, span }
    }
}
