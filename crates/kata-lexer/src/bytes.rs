//! Byte string literals — `b"Hello"`, `b'\x00\xFF'`.

use kata_ast::{Token, TokenWithSpan};
use kata_diagnostics::{FrontendError, MietteSpan};

use crate::{Lexer, Pos};

/// Lexa um byte string literal: `b"..."` ou `b'...'`.
///
/// O `b` já foi consumido pelo dispatch. Esta função é chamada quando
/// o lexer vê `"` ou `'` após um `b` que foi reconhecido como prefixo de bytes.
/// Aspas duplas e simples são equivalentes (mesma semântica que Text).
///
/// Escapes suportados: `\xNN` (hex byte), `\\`, `\"`, `\'`, `\n`, `\t`, `\r`, `\0`.
/// Qualquer byte 0x00-0xFF é aceito.
pub(crate) fn lex_bytes_string(
    lex: &mut Lexer,
    start: &Pos,
) -> Result<TokenWithSpan, FrontendError> {
    // Aspa de abertura já é o char atual — ler e consumir
    let quote = lex.ch.expect("lex_bytes_string chamado sem aspa");
    lex.advance(); // consome aspa de abertura

    let mut bytes: Vec<u8> = Vec::new();

    loop {
        match lex.ch {
            None | Some('\n') => {
                return Err(FrontendError::UnterminatedString {
                    span: MietteSpan(lex.span_from(start)),
                });
            }
            Some('\\') => {
                lex.advance(); // consome `\`
                match lex.ch {
                    Some('x') => {
                        lex.advance(); // consome `x`
                        // Espera exatamente 2 hex digits
                        let hi = lex.ch;
                        lex.advance();
                        let lo = lex.ch;
                        lex.advance();
                        match (hi, lo) {
                            (Some(h), Some(l))
                                if h.is_ascii_hexdigit() && l.is_ascii_hexdigit() =>
                            {
                                let val =
                                    u8::from_str_radix(&format!("{h}{l}"), 16).map_err(|_| {
                                        FrontendError::UnterminatedString {
                                            span: MietteSpan(lex.span_from(start)),
                                        }
                                    })?;
                                bytes.push(val);
                            }
                            _ => {
                                return Err(FrontendError::UnterminatedString {
                                    span: MietteSpan(lex.span_from(start)),
                                });
                            }
                        }
                    }
                    Some('n') => {
                        bytes.push(b'\n');
                        lex.advance();
                    }
                    Some('t') => {
                        bytes.push(b'\t');
                        lex.advance();
                    }
                    Some('r') => {
                        bytes.push(b'\r');
                        lex.advance();
                    }
                    Some('0') => {
                        bytes.push(0);
                        lex.advance();
                    }
                    Some('\\') => {
                        bytes.push(b'\\');
                        lex.advance();
                    }
                    Some('"') => {
                        bytes.push(b'"');
                        lex.advance();
                    }
                    Some('\'') => {
                        bytes.push(b'\'');
                        lex.advance();
                    }
                    Some(ch) => {
                        // Escape desconhecido: preserva o byte crus
                        bytes.push(b'\\');
                        bytes.push(ch as u8);
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
                lex.advance(); // consome aspa de fechamento
                break;
            }
            Some(ch) => {
                // Chars ASCII vão direto; chars não-ASCII produzem múltiplos bytes UTF-8
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                bytes.extend_from_slice(s.as_bytes());
                lex.advance();
            }
        }
    }

    Ok(TokenWithSpan {
        token: Token::BytesLit(bytes),
        span: lex.span_from(start),
    })
}
