//! Dispatcher de tokens — decide qual lexer chamar baseado no primeiro char.

use kata_ast::{Token, TokenWithSpan};
use kata_diagnostics::FrontendError;

use crate::Lexer;
use crate::bytes::lex_bytes_string;
use crate::ident::lex_ident;
use crate::number::lex_number;
use crate::text::lex_string;

pub(crate) fn lex_token(lex: &mut Lexer, had_space: bool) -> Result<TokenWithSpan, FrontendError> {
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
            lex.advance(); // consome |
            if lex.ch == Some('>') {
                lex.advance();
                Token::PipeForward
            } else if lex.ch.is_some_and(|c| c.is_ascii_digit()) {
                // |N> — pipe limitado com literal int.
                // Mas | também é usado em set literals: {|1 2 3|}.
                // Peek ahead: só é pipe limitado se os dígitos são seguidos por `>`.
                let mut peek_idx = 0usize;
                while lex.peek_n(peek_idx).is_some_and(|c| c.is_ascii_digit()) {
                    peek_idx += 1;
                }
                if lex.peek_n(peek_idx) == Some('>') {
                    let mut limit = String::new();
                    while lex.ch.is_some_and(|c| c.is_ascii_digit()) {
                        limit.push(lex.ch.unwrap());
                        lex.advance();
                    }
                    lex.advance(); // consome >
                    Token::PipeLimit { limit }
                } else {
                    // Não é pipe limitado — é | (pipe/set delimiter).
                    Token::Pipe
                }
            } else if lex.ch.is_some_and(|c| c.is_alphabetic() || c == '_') {
                // |ident> — pipe limitado com variável Int.
                // Mas | também é operador de fallback: `Err(E|Text)`.
                // Precisa lookahead: só é pipe limitado se o ident é
                // seguido por `>`. Senão, é Pipe seguido de Ident normal.
                // Peek ahead: conta chars alphanuméricos e verifica se
                // o próximo char após eles é `>`.
                let mut peek_idx = 0usize;
                while lex.peek_n(peek_idx).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    peek_idx += 1;
                }
                if lex.peek_n(peek_idx) == Some('>') {
                    // É pipe limitado — consome o ident e o >.
                    let mut limit = String::new();
                    while lex.ch.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        limit.push(lex.ch.unwrap());
                        lex.advance();
                    }
                    lex.advance(); // consome >
                    Token::PipeLimit { limit }
                } else {
                    // É Pipe (fallback) — não consome o ident.
                    Token::Pipe
                }
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
                // `!>` — recebimento por canal CSP: "valor sai do canal"
                Token::RecvArrow
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
            // `.6` → float sem parte inteira, mas apenas com espaço antes
            // (sem espaço, `.` é dot-access: `tpl.0`, `pessoa.nome`)
            if had_space && lex.peek().is_some_and(|c| c.is_ascii_digit()) {
                return lex_number(lex, &start);
            }
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
            // `<!` — envio por canal CSP: "valor entra no canal"
            lex.advance(); // consumir <
            lex.advance(); // consumir !
            Token::SendArrow
        }
        'b' if lex.peek() == Some('"') => {
            // `b"` — byte string literal.
            lex.advance(); // consome `b`, agora `"` é o char atual.
            return lex_bytes_string(lex, &start);
        }
        'b' if lex.peek() == Some('\'') => {
            // `b'` — byte string literal com aspas simples (equivalente a `b"`).
            lex.advance(); // consome `b`, agora `'` é o char atual.
            return lex_bytes_string(lex, &start);
        }
        _ => return lex_ident(lex, &start),
    };

    Ok(TokenWithSpan {
        token,
        span: lex.span_from(&start),
    })
}
