use kata_ast::{Token, TokenWithSpan};

/// Extrai apenas os tokens (sem span) de uma lista de TokenWithSpan.
pub(crate) fn tokens_only(tws: &[TokenWithSpan]) -> Vec<Token> {
    tws.iter().map(|t| t.token.clone()).collect()
}
