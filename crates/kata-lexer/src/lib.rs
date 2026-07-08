//! Analisador léxico indent-sensitive.
//!
//! Converte texto fonte em `Vec<TokenWithSpan>`. Emite tokens sintéticos
//! INDENT/DEDENT para o parser tratar blocos por indentação.
//!
//! Notação prefixa (I1): `+1` é número positivo, `+ 1` é a função `+`
//! aplicada a `1`. Operadores (`+`, `-`, `*`, `/`, `<`, `>`, `=`, `$`)
//! são identificadores como qualquer outro — o lexer não precisa de
//! contexto para decidir.

use std::iter::Peekable;
use std::str::Chars;

use kata_ast::{Span, Token, TokenWithSpan};
use kata_diagnostics::{FrontendError, MietteSpan};

// ── Posição salva para criação de Span ─────────────────────────

struct Pos {
    offset: usize,
    line: usize,
    col: usize,
}

// ── Lexer (char-by-char scanner) ───────────────────────────────

struct Lexer<'a> {
    source: &'a str,
    chars: Peekable<Chars<'a>>,
    pos: usize,
    line: usize,
    col: usize,
    ch: Option<char>,
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

    fn advance(&mut self) {
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

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.chars.clone().nth(n)
    }

    fn save_pos(&self) -> Pos {
        Pos {
            offset: self.pos,
            line: self.line,
            col: self.col,
        }
    }

    fn span_from(&self, start: &Pos) -> Span {
        Span::new(start.offset, start.line, start.col, self.pos - start.offset)
    }
}

// ── Resultado do processamento de indentação ──────────────────

enum IndentResult {
    Content,
    Eof,
}

/// Pula linhas em branco e comentários, conta indentação da próxima
/// linha com conteúdo, e emite INDENT/DEDENT conforme a pilha.
fn process_indent(
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
            while indent < *indent_stack.last().unwrap() {
                indent_stack.pop();
                tokens.push(TokenWithSpan {
                    token: Token::Dedent,
                    span: Span::synthetic(),
                });
            }
            let new_current = *indent_stack.last().unwrap();
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

// ── API pública ───────────────────────────────────────────────

/// Lexa o texto fonte e retorna `Vec<TokenWithSpan>`.
///
/// Emite tokens sintéticos INDENT/DEDENT/StmtSep para delimitar blocos
/// por indentação. Newlines dentro de parênteses/colchetes/chaves são
/// suprimidos (não geram StmtSep nem afetam indentação).
pub fn lex(source: &str) -> Result<Vec<TokenWithSpan>, FrontendError> {
    let mut lex = Lexer::new(source);
    let mut tokens: Vec<TokenWithSpan> = Vec::new();
    let mut indent_stack: Vec<usize> = vec![0];
    let mut bracket_depth: usize = 0;
    let mut line_has_content = false;

    // Processa indentação inicial (pula linhas vazias/comentários)
    match process_indent(&mut lex, &mut indent_stack, &mut tokens)? {
        IndentResult::Eof => {
            tokens.push(TokenWithSpan {
                token: Token::Eof,
                span: Span::zero(),
            });
            return Ok(tokens);
        }
        IndentResult::Content => {}
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
            continue; // volta ao topo — vai cair no \n ou EOF
        }

        // Newline
        if lex.ch == Some('\n') {
            lex.advance();
            if bracket_depth == 0 {
                let tokens_before = tokens.len();
                match process_indent(&mut lex, &mut indent_stack, &mut tokens)? {
                    IndentResult::Content => {
                        let indent_changed = tokens.len() > tokens_before;
                        if !indent_changed && line_has_content {
                            tokens.push(TokenWithSpan {
                                token: Token::StmtSep,
                                span: Span::synthetic(),
                            });
                        }
                        line_has_content = false;
                    }
                    IndentResult::Eof => break,
                }
            }
            // Se bracket_depth > 0: newline suprimido, continua
            continue;
        }

        // EOF
        if lex.ch.is_none() {
            break;
        }

        // Lexa um token real
        let token = lex_token(&mut lex)?;
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

    Ok(tokens)
}

// ── Dispatcher de tokens ───────────────────────────────────────

fn lex_token(lex: &mut Lexer) -> Result<TokenWithSpan, FrontendError> {
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
                // `=` não seguido de `>` → identificador
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
            Token::Bang
        }
        '@' => {
            lex.advance();
            Token::At
        }
        '.' => {
            lex.advance();
            Token::Dot
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
        _ => return lex_ident(lex, &start),
    };

    Ok(TokenWithSpan {
        token,
        span: lex.span_from(&start),
    })
}

// ── Identificadores e palavras-chave ───────────────────────────

fn lex_ident(lex: &mut Lexer, start: &Pos) -> Result<TokenWithSpan, FrontendError> {
    loop {
        match lex.ch {
            None => break,
            Some(' ') | Some('\t') | Some('\r') | Some('\n') => break,
            Some('(') | Some(')') | Some('[') | Some(']') | Some('{') | Some('}') | Some(',')
            | Some(';') | Some('.') => break,
            Some('#') => break,
            Some(':') | Some('|') | Some('?') | Some('!') | Some('@') => break,
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
        "with" => Token::With,
        "match" => Token::Match,
        "return" => Token::Return,
        "otherwise" => Token::Otherwise,
        _ => Token::Ident(ident.to_string()),
    };

    Ok(TokenWithSpan {
        token,
        span: lex.span_from(start),
    })
}

// ── Literais numéricos ─────────────────────────────────────────

/// Lexa um literal numérico (inteiro ou float).
///
/// Suporta bases: 10 (default), 16 (0x), 8 (0o), 2 (0b), 10 explícito (0d).
/// Separador `_` é descartado léxicamente.
/// Floats: `3.14`, `1.5e10`, `1.5E-10`, `1e10`.
///
/// O texto bruto é preservado (com prefixo de base, sem underscores)
/// para o runtime fazer parsing de BigInt/SMI.
fn lex_number(lex: &mut Lexer, start: &Pos) -> Result<TokenWithSpan, FrontendError> {
    let mut base: u32 = 10;
    let mut has_digits = false;

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

// ── Literais de string ────────────────────────────────────────

/// Lexa uma string literal.
///
/// Suporta: aspas duplas ("..."), aspas simples ('...'), triplas ("""...""").
/// Aspas duplas e simples aceitam escape sequences (\\n, \\t, \\\\, etc).
/// Triplas são raw (sem escape processing), multilinha.
fn lex_string(lex: &mut Lexer, start: &Pos) -> Result<TokenWithSpan, FrontendError> {
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
