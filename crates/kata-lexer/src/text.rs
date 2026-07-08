//! Literais de string — aspas duplas, simples, triplas (raw multilinha).

use kata_ast::{Token, TokenWithSpan};
use kata_diagnostics::{FrontendError, MietteSpan};

use crate::{Lexer, Pos};

/// Lexa uma string literal.
///
/// Suporta: aspas duplas ("..."), aspas simples ('...'), triplas ("""...""").
/// Aspas duplas e simples aceitam escape sequences (\\n, \\t, \\\\, etc).
/// Triplas são raw (sem escape processing), multilinha.
pub(crate) fn lex_string(lex: &mut Lexer, start: &Pos) -> Result<TokenWithSpan, FrontendError> {
    let quote = lex.ch.expect("lex_string chamado sem aspas");
    lex.advance(); // consome aspa de abertura

    // String tripla (raw, multilinha) — apenas para aspas duplas
    if quote == '"' && lex.ch == Some('"') && lex.peek() == Some('"') {
        lex.advance(); // segunda "
        lex.advance(); // terceira "
        let content_start = lex.pos;
        loop {
            let is_triple =
                lex.ch == Some('"') && lex.peek() == Some('"') && lex.peek_n(1) == Some('"');
            match lex.ch {
                None => {
                    return Err(FrontendError::UnterminatedString {
                        span: MietteSpan(lex.span_from(start)),
                    });
                }
                Some('"') if is_triple => {
                    lex.advance(); // primeira "
                    lex.advance(); // segunda "
                    lex.advance(); // terceira "
                    break;
                }
                _ => lex.advance(),
            }
        }
        let content = lex.source[content_start..lex.pos - 3].to_string();
        return Ok(TokenWithSpan {
            token: Token::TextLit(content),
            span: lex.span_from(start),
        });
    }

    // String normal (com escapes)
    let mut text = String::new();
    loop {
        match lex.ch {
            None | Some('\n') => {
                return Err(FrontendError::UnterminatedString {
                    span: MietteSpan(lex.span_from(start)),
                });
            }
            Some('\\') => {
                lex.advance();
                match lex.ch {
                    Some('n') => {
                        text.push('\n');
                        lex.advance();
                    }
                    Some('t') => {
                        text.push('\t');
                        lex.advance();
                    }
                    Some('\\') => {
                        text.push('\\');
                        lex.advance();
                    }
                    Some('"') => {
                        text.push('"');
                        lex.advance();
                    }
                    Some('\'') => {
                        text.push('\'');
                        lex.advance();
                    }
                    Some('r') => {
                        text.push('\r');
                        lex.advance();
                    }
                    Some('0') => {
                        text.push('\0');
                        lex.advance();
                    }
                    Some(ch) => {
                        // Escape desconhecido: preserva ambos
                        text.push('\\');
                        text.push(ch);
                        lex.advance();
                    }
                    None => {
                        return Err(FrontendError::UnterminatedString {
                            span: MietteSpan(lex.span_from(start)),
                        });
                    }
                }
            }
            Some(ch) if ch == quote => {
                lex.advance();
                break;
            }
            Some(ch) => {
                text.push(ch);
                lex.advance();
            }
        }
    }

    Ok(TokenWithSpan {
        token: Token::TextLit(text),
        span: lex.span_from(start),
    })
}
