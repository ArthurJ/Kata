//! Processamento de indentação — pula linhas vazias/comentários,
//! conta indentação, emite INDENT/DEDENT conforme a pilha.

use kata_ast::{Span, Token, TokenWithSpan};
use kata_diagnostics::{FrontendError, MietteSpan};

use crate::Lexer;

/// Resultado do processamento de indentação.
pub(crate) enum IndentResult {
    Content,
    Eof,
}

/// Pula linhas em branco e comentários, conta indentação da próxima
/// linha com conteúdo, e emite INDENT/DEDENT conforme a pilha.
pub(crate) fn process_indent(
    lex: &mut Lexer,
    indent_stack: &mut Vec<usize>,
    tokens: &mut Vec<TokenWithSpan>,
) -> Result<IndentResult, FrontendError> {
    loop {
        let mut indent = 0;
        loop {
            match lex.ch {
                Some(' ') | Some('\t') => {
                    indent += 1;
                    lex.advance();
                }
                Some('\r') => {
                    lex.advance();
                }
                Some('#') => {
                    // Linha de comentário — pula até \n inclusive
                    while lex.ch.is_some() && lex.ch != Some('\n') {
                        lex.advance();
                    }
                    if lex.ch == Some('\n') {
                        lex.advance();
                    }
                    indent = 0;
                    continue; // reinicia contagem na próxima linha
                }
                Some('\n') => {
                    // Linha em branco — pula
                    lex.advance();
                    indent = 0;
                    continue;
                }
                None => return Ok(IndentResult::Eof),
                _ => break, // conteúdo encontrado
            }
        }

        // Conteúdo encontrado neste nível de indentação
        let current = *indent_stack.last().expect("indent_stack não vazia");
        if indent > current {
            indent_stack.push(indent);
            tokens.push(TokenWithSpan {
                token: Token::Indent,
                span: Span::synthetic(),
            });
        } else if indent < current {
            while indent < *indent_stack.last().expect("indent_stack não vazia") {
                indent_stack.pop();
                tokens.push(TokenWithSpan {
                    token: Token::Dedent,
                    span: Span::synthetic(),
                });
            }
            let new_current = *indent_stack.last().expect("indent_stack não vazia");
            if indent != new_current {
                return Err(FrontendError::InconsistentIndent {
                    expected: new_current,
                    found: indent,
                    span: MietteSpan(Span::new(lex.pos, lex.line, lex.col, 1)),
                });
            }
        }
        return Ok(IndentResult::Content);
    }
}
