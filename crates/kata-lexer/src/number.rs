//! Literais numéricos — inteiros (multi-base) e floats.

use kata_ast::{Token, TokenWithSpan};
use kata_diagnostics::{FrontendError, MietteSpan};

use crate::{Lexer, Pos};

/// Lexa um literal numérico (inteiro ou float).
///
/// Suporta bases: 10 (default), 16 (0x), 8 (0o), 2 (0b), 10 explícito (0d).
/// Separador `_` é descartado léxicamente.
/// Floats: `3.14`, `1.5e10`, `1.5E-10`, `1e10`, `.6` (→ `0.6`).
///
/// O texto bruto é preservado (com prefixo de base, sem underscores)
/// para o runtime fazer parsing de BigInt/SMI.
pub(crate) fn lex_number(lex: &mut Lexer, start: &Pos) -> Result<TokenWithSpan, FrontendError> {
    let mut base: u32 = 10;
    let mut has_digits = false;

    // Float sem parte inteira: `.6` → normaliza para `0.6`
    if lex.ch == Some('.') {
        lex.advance(); // consome '.'
        while lex.ch.is_some_and(|c| c.is_ascii_digit() || c == '_') {
            if lex.ch != Some('_') {
                has_digits = true;
            }
            lex.advance();
        }
        if !has_digits {
            return Err(FrontendError::InvalidNumber {
                text: lex.source[start.offset..lex.pos].to_string(),
                span: MietteSpan(lex.span_from(start)),
            });
        }
        // Expoente opcional
        if lex.ch == Some('e') || lex.ch == Some('E') {
            lex.advance();
            if lex.ch == Some('-') || lex.ch == Some('+') {
                lex.advance();
            }
            if !lex.ch.is_some_and(|c| c.is_ascii_digit()) {
                return Err(FrontendError::InvalidNumber {
                    text: lex.source[start.offset..lex.pos].to_string(),
                    span: MietteSpan(lex.span_from(start)),
                });
            }
            while lex.ch.is_some_and(|c| c.is_ascii_digit()) {
                lex.advance();
            }
        }
        let raw = &lex.source[start.offset..lex.pos];
        let text: String = format!("0{}", raw.chars().filter(|c| *c != '_').collect::<String>());
        return Ok(TokenWithSpan {
            token: Token::FloatLit(text),
            span: lex.span_from(start),
        });
    }

    // Verifica prefixo de base (apenas se o primeiro char for '0')
    if lex.ch == Some('0') {
        lex.advance();
        match lex.ch {
            Some('x') | Some('X') => {
                base = 16;
                lex.advance();
            }
            Some('o') | Some('O') => {
                base = 8;
                lex.advance();
            }
            Some('b') | Some('B') => {
                base = 2;
                lex.advance();
            }
            Some('d') | Some('D') => {
                base = 10;
                lex.advance();
            }
            _ => {
                // `0` sozinho ou seguido de não-dígito — já consumido
                has_digits = true;
            }
        }
    }

    // Consome dígitos e underscores
    loop {
        match lex.ch {
            Some('_') => {
                lex.advance();
            }
            Some(ch) if ch.is_digit(base) => {
                has_digits = true;
                lex.advance();
            }
            _ => break,
        }
    }

    if !has_digits {
        return Err(FrontendError::InvalidNumber {
            text: lex.source[start.offset..lex.pos].to_string(),
            span: MietteSpan(lex.span_from(start)),
        });
    }

    // Float (apenas base 10)
    if base == 10 {
        // Ponto decimal seguido de dígito → float
        if lex.ch == Some('.') && lex.peek().is_some_and(|c| c.is_ascii_digit()) {
            lex.advance(); // consome '.'
            loop {
                match lex.ch {
                    Some('_') => {
                        lex.advance();
                    }
                    Some(ch) if ch.is_ascii_digit() => {
                        lex.advance();
                    }
                    _ => break,
                }
            }
            // Expoente opcional
            if lex.ch == Some('e') || lex.ch == Some('E') {
                lex.advance();
                if lex.ch == Some('-') || lex.ch == Some('+') {
                    lex.advance();
                }
                if !lex.ch.is_some_and(|c| c.is_ascii_digit()) {
                    return Err(FrontendError::InvalidNumber {
                        text: lex.source[start.offset..lex.pos].to_string(),
                        span: MietteSpan(lex.span_from(start)),
                    });
                }
                while lex.ch.is_some_and(|c| c.is_ascii_digit()) {
                    lex.advance();
                }
            }
            let raw = &lex.source[start.offset..lex.pos];
            let text: String = raw.chars().filter(|c| *c != '_').collect();
            return Ok(TokenWithSpan {
                token: Token::FloatLit(text),
                span: lex.span_from(start),
            });
        }

        // Expoente sem ponto decimal → float
        if lex.ch == Some('e') || lex.ch == Some('E') {
            lex.advance();
            if lex.ch == Some('-') || lex.ch == Some('+') {
                lex.advance();
            }
            if !lex.ch.is_some_and(|c| c.is_ascii_digit()) {
                return Err(FrontendError::InvalidNumber {
                    text: lex.source[start.offset..lex.pos].to_string(),
                    span: MietteSpan(lex.span_from(start)),
                });
            }
            while lex.ch.is_some_and(|c| c.is_ascii_digit()) {
                lex.advance();
            }
            let raw = &lex.source[start.offset..lex.pos];
            let text: String = raw.chars().filter(|c| *c != '_').collect();
            return Ok(TokenWithSpan {
                token: Token::FloatLit(text),
                span: lex.span_from(start),
            });
        }
    }

    // Inteiro
    let raw = &lex.source[start.offset..lex.pos];
    let text: String = raw.chars().filter(|c| *c != '_').collect();
    Ok(TokenWithSpan {
        token: Token::IntLit(text),
        span: lex.span_from(start),
    })
}
