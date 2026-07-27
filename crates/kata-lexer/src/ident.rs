//! Identificadores e palavras-chave.

use kata_ast::{Token, TokenWithSpan};
use kata_diagnostics::FrontendError;

use crate::{Lexer, Pos};

pub(crate) fn lex_ident(lex: &mut Lexer, start: &Pos) -> Result<TokenWithSpan, FrontendError> {
    // Determina se o identificador é alfanumérico (começa com letra ou _)
    // ou simbólico (começa com operador como <, >, +, -, *, /, =).
    // Para identificadores alfanuméricos, `=` é break char (para suportar
    // `E=Text` em defaults de type params). Para simbólicos, `=` não é
    // break (para suportar `<=`, `>=`, `!=`).
    let is_alpha = lex.ch.is_some_and(|c| c.is_alphabetic() || c == '_');

    loop {
        match lex.ch {
            None => break,
            Some(' ') | Some('\t') | Some('\r') | Some('\n') => break,
            Some('(') | Some(')') | Some('[') | Some(']') | Some('{') | Some('}') | Some(',')
            | Some(';') | Some('.') => break,
            Some('#') => break,
            Some(':') | Some('|') | Some('?') | Some('!') | Some('@') => break,
            Some('=') if is_alpha => break,
            _ => lex.advance(),
        }
    }

    let ident = &lex.source[start.offset..lex.pos];

    let token = match ident {
        "let" => Token::Let,
        "var" => Token::Var,
        "data" => Token::Data,
        "enum" => Token::Enum,
        "alias" => Token::Alias,
        "action" => Token::Action,
        "lambda" => Token::Lambda,
        "import" => Token::Import,
        "export" => Token::Export,
        "as" => Token::As,
        "interface" => Token::Interface,
        "implements" => Token::Implements,
        "refines" => Token::Refines,
        "with" => Token::With,
        "match" => Token::Match,
        "return" => Token::Return,
        "loop" => Token::Loop,
        "break" => Token::Break,
        "continue" => Token::Continue,
        "otherwise" => Token::Otherwise,
        "for" => Token::For,
        "in" => Token::In,
        "select" => Token::Select,
        "timeout" => Token::Timeout,
        "type" => Token::Type,
        _ => Token::Ident(ident.to_string()),
    };

    Ok(TokenWithSpan {
        token,
        span: lex.span_from(start),
    })
}
