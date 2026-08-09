//! Analisador léxico indent-sensitive.
//!
//! Converte texto fonte em `Vec<TokenWithSpan>`. Emite tokens sintéticos
//! INDENT/DEDENT para o parser tratar blocos por indentação.
//!
//! Notação prefixa (I1): `+1` é número positivo, `+ 1` é a função `+`
//! aplicada a `1`. Operadores (`+`, `-`, `*`, `/`, `<`, `>`, `=`, `$`)
//! são identificadores como qualquer outro — o lexer não precisa de
//! contexto para decidir.

mod bytes;
mod dispatch;
mod ident;
mod indent;
mod number;
mod text;

use std::iter::Peekable;
use std::str::Chars;

use kata_ast::{Span, Token, TokenWithSpan};
use kata_diagnostics::FrontendError;

use indent::{IndentResult, process_indent};

// ── Posição salva para criação de Span ─────────────────────────

pub(crate) struct Pos {
    pub(crate) offset: usize,
    pub(crate) line: usize,
    pub(crate) col: usize,
}

// ── Lexer (char-by-char scanner) ───────────────────────────────

pub(crate) struct Lexer<'a> {
    pub(crate) source: &'a str,
    pub(crate) chars: Peekable<Chars<'a>>,
    pub(crate) pos: usize,
    pub(crate) line: usize,
    pub(crate) col: usize,
    pub(crate) ch: Option<char>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        let mut chars = source.chars().peekable();
        let ch = chars.next();
        Self {
            source,
            chars,
            pos: 0,
            line: 1,
            col: 1,
            ch,
        }
    }

    pub(crate) fn advance(&mut self) {
        if let Some(ch) = self.ch {
            self.pos += ch.len_utf8();
            if ch == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += ch.len_utf8();
            }
            self.ch = self.chars.next();
        }
    }

    pub(crate) fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    pub(crate) fn peek_n(&self, n: usize) -> Option<char> {
        self.chars.clone().nth(n)
    }

    pub(crate) fn save_pos(&self) -> Pos {
        Pos {
            offset: self.pos,
            line: self.line,
            col: self.col,
        }
    }

    pub(crate) fn span_from(&self, start: &Pos) -> Span {
        Span::new(start.offset, start.line, start.col, self.pos - start.offset)
    }
}

// ── Recovery helper ───────────────────────────────────────────

/// Consome caracteres até o próximo ponto de sincronização: `\n`, `;`, ou EOF.
///
/// Usado por `lex_with_recovery` para recuperar de erros léxicos.
/// Para no `\n` (deixa o newline como char atual para o loop principal
/// processar) ou no `;` (consome o `;` para não gerar token lixo).
fn skip_to_sync(lex: &mut Lexer) {
    while let Some(ch) = lex.ch {
        if ch == '\n' {
            // Não consome o \n — deixa o loop principal processá-lo
            return;
        }
        if ch == ';' {
            lex.advance(); // consome o ; para não virar token
            return;
        }
        lex.advance();
    }
}

// ── API pública ───────────────────────────────────────────────

/// Lexa o texto fonte e retorna `Vec<TokenWithSpan>`.
///
/// Emite tokens sintéticos INDENT/DEDENT/StmtSep para delimitar blocos
/// por indentação. Newlines dentro de parênteses/colchetes/chaves são
/// suprimidos (não geram StmtSep nem afetam indentação).
///
/// Aborta no primeiro erro léxico. Para múltiplos erros em uma única
/// passada, use [`lex_with_recovery`].
pub fn lex(source: &str) -> Result<Vec<TokenWithSpan>, FrontendError> {
    let (tokens, errors) = lex_with_recovery(source);
    if let Some(first) = errors.into_iter().next() {
        return Err(first);
    }
    Ok(tokens)
}

/// Lexa o texto fonte com recovery de erros léxicos.
///
/// Análoga a [`lex`], mas não aborta no primeiro erro. Quando um erro
/// léxico é encontrado (`UnterminatedString`, `InvalidNumber`,
/// `InconsistentIndent`), o erro é registrado e o lexer skipa caracteres
/// até o próximo ponto de sincronização (`\n` ou `;`), então continua
/// tokenizando o resto do fonte.
///
/// Retorna sempre `(tokens, errors)`:
/// - `tokens`: tokens válidos (pode ser vazio se houver erros graves)
/// - `errors`: erros léxicos encontrados (vazio se tudo ok)
///
/// Se há erros, os tokens são parciais — linhas com erro são descartadas.
/// O caller deve decidir se continua para parse (all-or-nothing por fase)
/// ou aborta.
pub fn lex_with_recovery(source: &str) -> (Vec<TokenWithSpan>, Vec<FrontendError>) {
    let mut lex = Lexer::new(source);
    let mut tokens: Vec<TokenWithSpan> = Vec::new();
    let mut errors: Vec<FrontendError> = Vec::new();
    let mut indent_stack: Vec<usize> = vec![0];
    let mut bracket_depth: usize = 0;
    let mut line_has_content = false;

    // Processa indentação inicial (pula linhas vazias/comentários)
    match process_indent(&mut lex, &mut indent_stack, &mut tokens) {
        Ok(IndentResult::Eof) => {
            tokens.push(TokenWithSpan {
                token: Token::Eof,
                span: Span::zero(),
            });
            return (tokens, errors);
        }
        Ok(IndentResult::Content) => {}
        Err(e) => {
            errors.push(e);
            skip_to_sync(&mut lex);
        }
    }

    loop {
        // Pula whitespace horizontal
        while matches!(lex.ch, Some(' ') | Some('\t') | Some('\r')) {
            lex.advance();
        }

        // Comentário: pula até \n ou EOF
        if lex.ch == Some('#') {
            while lex.ch.is_some() && lex.ch != Some('\n') {
                lex.advance();
            }
            continue;
        }

        // Newline
        if lex.ch == Some('\n') {
            lex.advance();
            if bracket_depth == 0 {
                let tokens_before = tokens.len();
                match process_indent(&mut lex, &mut indent_stack, &mut tokens) {
                    Ok(IndentResult::Content) => {
                        let indent_changed = tokens.len() > tokens_before;
                        if !indent_changed && line_has_content {
                            tokens.push(TokenWithSpan {
                                token: Token::StmtSep,
                                span: Span::synthetic(),
                            });
                        }
                        line_has_content = false;
                    }
                    Ok(IndentResult::Eof) => break,
                    Err(e) => {
                        errors.push(e);
                        skip_to_sync(&mut lex);
                        line_has_content = false;
                    }
                }
            }
            continue;
        }

        // EOF
        if lex.ch.is_none() {
            break;
        }

        // Lexa um token real
        match dispatch::lex_token(&mut lex) {
            Ok(token) => {
                line_has_content = true;
                // Atualiza profundidade de brackets
                match &token.token {
                    Token::LParen | Token::LBracket | Token::LBrace => bracket_depth += 1,
                    Token::RParen | Token::RBracket | Token::RBrace => {
                        bracket_depth = bracket_depth.saturating_sub(1)
                    }
                    _ => {}
                }
                tokens.push(token);
            }
            Err(e) => {
                errors.push(e);
                skip_to_sync(&mut lex);
            }
        }
    }

    // Esvazia pilha de indentação — emite DEDENTs restantes
    while indent_stack.len() > 1 {
        indent_stack.pop();
        tokens.push(TokenWithSpan {
            token: Token::Dedent,
            span: Span::synthetic(),
        });
    }

    tokens.push(TokenWithSpan {
        token: Token::Eof,
        span: Span::zero(),
    });

    (tokens, errors)
}
