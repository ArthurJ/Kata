//! Dispatcher de tokens — decide qual lexer chamar baseado no primeiro char.

use kata_ast::{Token, TokenWithSpan};
use kata_diagnostics::FrontendError;

use crate::Lexer;
use crate::bytes::lex_bytes_string;
use crate::ident::lex_ident;
use crate::number::lex_number;
use crate::text::lex_string;

pub(crate) fn lex_token(lex: &mut Lexer) -> Result<TokenWithSpan, FrontendError> {
    let start = lex.save_pos();
    let ch = lex.ch.expect("lex_token chamado no EOF");

    // ── Números (incluindo sinais + e -) ──
    if ch.is_ascii_digit() {
        return lex_number(lex, &start);
    }
    if (ch == '+' || ch == '-') && lex.peek().is_some_and(|c| c.is_ascii_digit()) {
        lex.advance(); // consome sinal
        return lex_number(lex, &start);
    }

    // ── Strings ──
    if ch == '"' || ch == '\'' {
        return lex_string(lex, &start);
    }

    // ── λ → Lambda ──
    if ch == 'λ' {
        lex.advance();
        return Ok(TokenWithSpan {
            token: Token::Lambda,
            span: lex.span_from(&start),
        });
    }

    // ── Operadores multi-char e pontuação ──
    let token = match ch {
        ':' => {
            lex.advance();
            match lex.ch {
                Some(':') => {
                    lex.advance();
                    Token::DoubleColon
                }
                Some('=') => {
                    lex.advance();
                    Token::BindAssign
                }
                _ => Token::Colon,
            }
        }
        '=' => {
            if lex.peek() == Some('>') {
                lex.advance();
                lex.advance();
                Token::FatArrow
            } else {
                // `=` não seguido de `>` → identificador (operador de igualdade)
                return lex_ident(lex, &start);
            }
        }
        '-' => {
            if lex.peek() == Some('>') {
                lex.advance();
                lex.advance();
                Token::ThinArrow
            } else {
                // `-` não seguido de `>` ou dígito → identificador
                return lex_ident(lex, &start);
            }
        }
        '|' => {
            lex.advance();
            if lex.ch == Some('>') {
                lex.advance();
                Token::PipeForward
            } else {
                Token::Pipe
            }
        }
        '?' => {
            lex.advance();
            Token::Question
        }
        '!' => {
            lex.advance();
            if lex.ch == Some('>') {
                lex.advance();
                Token::SendArrow
            } else if lex.ch == Some('=') {
                // `!=` — operador de desigualdade. Lexar via lex_ident
                // (identificador simbólico, mesmo mecanismo de `<`, `>`, `=`).
                lex.advance();
                return lex_ident(lex, &start);
            } else {
                Token::Bang
            }
        }
        '@' => {
            lex.advance();
            Token::At
        }
        '.' => {
            lex.advance();
            if lex.ch == Some('.') {
                // `..` — pode ser `..=` ou `..`
                lex.advance();
                if lex.ch == Some('=') {
                    lex.advance();
                    Token::DotDotEq
                } else {
                    Token::DotDot
                }
            } else {
                Token::Dot
            }
        }
        ';' => {
            lex.advance();
            Token::Semicolon
        }
        ',' => {
            lex.advance();
            Token::Comma
        }
        '(' => {
            lex.advance();
            Token::LParen
        }
        ')' => {
            lex.advance();
            Token::RParen
        }
        '[' => {
            lex.advance();
            Token::LBracket
        }
        ']' => {
            lex.advance();
            Token::RBracket
        }
        '{' => {
            lex.advance();
            Token::LBrace
        }
        '}' => {
            lex.advance();
            Token::RBrace
        }
        '<' if lex.peek() == Some('!') => {
            // `<` seguido de `!` → RecvArrow.
            lex.advance(); // consumir <
            lex.advance(); // consumir !
            Token::RecvArrow
        }
        'b' if lex.peek() == Some('"') => {
            // `b"` — byte string literal.
            lex.advance(); // consome `b`, agora `"` é o char atual.
            return lex_bytes_string(lex, &start);
        }
        _ => return lex_ident(lex, &start),
    };

    Ok(TokenWithSpan {
        token,
        span: lex.span_from(&start),
    })
}
