use crate::parser::ast::Span;

#[derive(Debug, Clone)]
pub struct OptimizerError {
    pub message: String,
    pub _span: Span,
}

impl OptimizerError {
    pub fn new(message: String, span: Span) -> Self {
        Self { message, _span: span }
    }
}
