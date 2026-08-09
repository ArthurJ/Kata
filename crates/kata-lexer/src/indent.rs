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
        // Pre-check: encontra o nível da pilha que casaria com indent,
        // sem mutar a pilha. Se nenhum casa, é inconsistente.
        let mut target_idx = indent_stack.len() - 1;
        while target_idx > 0 && indent < indent_stack[target_idx] {
            target_idx -= 1;
        }
        if indent != indent_stack[target_idx] {
            return Err(FrontendError::InconsistentIndent {
                expected: indent_stack[target_idx],
                found: indent,
                span: MietteSpan(Span::new(lex.pos, lex.line, lex.col, 1)),
            });
        }
        // Commit: emite DEDENTs para cada nível removido
        while indent_stack.len() - 1 > target_idx {
            indent_stack.pop();
            tokens.push(TokenWithSpan {
                token: Token::Dedent,
                span: Span::synthetic(),
            });
        }
    }
    Ok(IndentResult::Content)
}
