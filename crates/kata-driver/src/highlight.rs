//! Syntax highlighting para o REPL — coloriza o input em tempo real.
//!
//! Usa o lexer do próprio Kata5 para tokenizar a linha e mapeia cada `Token`
//! para uma cor ANSI. O mapeamento segue a mesma categorização do
//! `Kata.tmbundle/Syntaxes/Kata.tmLanguage.json`:
//!
//! - Palavras-chave de declaração (`let`, `data`, `enum`, `action`, ...): bold cyan
//! - Palavras-chave de controle (`match`, `return`, `loop`, ...): bold yellow
//! - Palavras-chave de módulo (`import`, `export`, `as`): bold magenta
//! - Palavras-chave de tipo (`interface`, `implements`, `refines`, `with`): bold blue
//! - `lambda`/`λ`, `type`: bold green
//! - Literais (Int, Float, Text): green
//! - Diretivas (`@ffi`, `@builtin`, ...): bright magenta
//! - Operadores simbólicos (`+`, `-`, `*`, `/`, `<`, `>`, `=`): bright yellow
//! - Operadores sintáticos (`:=`, `::`, `=>`, `->`, `|>`, `!>`, `<!`, `?`, `!`, `|`): bright black (gray)
//! - Pontuação (`(`, `)`, `[`, `]`, `{`, `}`, `,`, `;`, `:`, `.`): gray
//! - Identificadores PascalCase (tipos/construtores): bright blue
//! - Identificadores comuns: sem cor (default)
//!
//! O highlighter é tolerante a falhas: se o lexer falha (input incompleto),
//! retorna a linha original sem colorir.

use std::borrow::Cow::{self, Borrowed, Owned};

use kata_ast::Token;
use kata_lexer::lex;
use rustyline::CompletionType;
use rustyline::Context;
use rustyline::Result as RlResult;
use rustyline::completion::Completer;
use rustyline::highlight::CmdKind;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;

// ── Códigos ANSI ────────────────────────────────────────────
const RESET: &str = "\x1b[0m";
const BOLD_CYAN: &str = "\x1b[1;36m";
const BOLD_YELLOW: &str = "\x1b[1;33m";
const BOLD_MAGENTA: &str = "\x1b[1;35m";
const BOLD_BLUE: &str = "\x1b[1;34m";
const BOLD_GREEN: &str = "\x1b[1;32m";
const GREEN: &str = "\x1b[32m";
const BRIGHT_MAGENTA: &str = "\x1b[95m";
const BRIGHT_YELLOW: &str = "\x1b[93m";
const BRIGHT_BLACK: &str = "\x1b[90m";
const BRIGHT_BLUE: &str = "\x1b[94m";

/// Mapeia um `Token` para sua cor ANSI (sem o reset).
///
/// Retorna `None` para identificadores comuns (sem cor — default do terminal).
fn token_color(token: &Token) -> Option<&'static str> {
    match token {
        // ── Palavras-chave de declaração ──────────────────
        Token::Let
        | Token::Var
        | Token::Data
        | Token::Enum
        | Token::Alias
        | Token::Action
        | Token::Directive => Some(BOLD_CYAN),

        // ── Palavras-chave de controle ────────────────────
        Token::Match
        | Token::Return
        | Token::Loop
        | Token::Break
        | Token::Continue
        | Token::Otherwise
        | Token::For
        | Token::In
        | Token::Select
        | Token::Timeout => Some(BOLD_YELLOW),

        // ── Palavras-chave de módulo ──────────────────────
        Token::Import | Token::Export | Token::As => Some(BOLD_MAGENTA),

        // ── Palavras-chave de tipo ────────────────────────
        Token::Interface | Token::Implements | Token::Refines | Token::With => Some(BOLD_BLUE),

        // ── lambda/λ, type ────────────────────────────────
        Token::Lambda | Token::Type => Some(BOLD_GREEN),

        // ── Literais ──────────────────────────────────────
        Token::IntLit(_) | Token::FloatLit(_) | Token::TextLit(_) | Token::BytesLit(_) => {
            Some(GREEN)
        }

        // ── Diretivas (@nome) ─────────────────────────────
        Token::At => Some(BRIGHT_MAGENTA),

        // ── Identificadores ───────────────────────────────
        Token::Ident(s) => {
            if is_symbolic_op(s) {
                Some(BRIGHT_YELLOW)
            } else if is_pascal_case(s) {
                Some(BRIGHT_BLUE)
            } else {
                None
            }
        }

        // ── Operadores sintáticos ─────────────────────────
        Token::BindAssign
        | Token::DoubleColon
        | Token::FatArrow
        | Token::ThinArrow
        | Token::PipeForward
        | Token::SendArrow
        | Token::RecvArrow
        | Token::Question
        | Token::Bang
        | Token::Pipe
        | Token::DotDot
        | Token::DotDotEq => Some(BRIGHT_BLACK),

        // ── Pontuação ──────────────────────────────────────
        Token::LParen
        | Token::RParen
        | Token::LBracket
        | Token::RBracket
        | Token::LBrace
        | Token::RBrace
        | Token::Comma
        | Token::Semicolon
        | Token::Colon
        | Token::Dot => Some(BRIGHT_BLACK),

        // ── Tokens sintéticos e EOF — não colorir ──────────
        Token::Indent | Token::Dedent | Token::StmtSep | Token::Eof => None,
    }
}

/// Verifica se um identificador é um operador simbólico.
fn is_symbolic_op(s: &str) -> bool {
    s.len() == 1 && matches!(s, "+" | "-" | "*" | "/" | "<" | ">" | "=")
}

/// Verifica se um identificador é PascalCase (primeira letra maiúscula).
fn is_pascal_case(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Tokeniza a linha e aplica cores ANSI.
fn highlight_line(line: &str) -> Cow<'_, str> {
    if line.is_empty() {
        return Borrowed(line);
    }

    let tokens = match lex(line) {
        Ok(t) => t,
        // Lexer falha em input incompleto — devolver sem cor.
        Err(_) => return Borrowed(line),
    };

    let mut result = String::with_capacity(line.len() + tokens.len() * 8);
    let mut last_end = 0usize;

    for tok in &tokens {
        // Tokens sintéticos (Eof, Indent, Dedent, StmtSep) têm span zero
        // (offset e len = 0). Processá-los regrediria `last_end` e faria
        // o "resto da linha" reimprimir todo o input sem cor — eco duplo.
        if tok.span.len == 0 {
            continue;
        }

        let tok_start = tok.span.offset;
        let tok_end = tok_start + tok.span.len;

        // Gap entre tokens (espaços, comentários que o lexer saltou).
        if tok_start > last_end {
            result.push_str(&line[last_end..tok_start]);
        }

        let end = tok_end.min(line.len());
        let text = &line[tok_start..end];

        match token_color(&tok.token) {
            Some(color) => {
                result.push_str(color);
                result.push_str(text);
                result.push_str(RESET);
            }
            None => {
                result.push_str(text);
            }
        }

        last_end = end;
    }

    // Resto da linha após o último token.
    if last_end < line.len() {
        result.push_str(&line[last_end..]);
    }

    Owned(result)
}

/// Helper do rustyline para o REPL Kata — combina highlighter com
/// hinter de histórico e validator (ambos sem customização).
///
/// O trait `Helper` exige `Completer + Hinter + Highlighter + Validator`.
/// Implementamos `Highlighter` com colorização baseada no lexer; os outros
/// três usam implementações padrão do rustyline.
pub(crate) struct KataHelper {
    completer: rustyline::completion::FilenameCompleter,
    #[allow(dead_code)]
    // HistoryHinter mantida como hook de integração rustyline; o impl Hinter retorna None (ver doc em `hint`).
    hinter: rustyline::hint::HistoryHinter,
}

impl Default for KataHelper {
    fn default() -> Self {
        Self {
            completer: rustyline::completion::FilenameCompleter::new(),
            hinter: rustyline::hint::HistoryHinter::new(),
        }
    }
}

impl Completer for KataHelper {
    type Candidate = rustyline::completion::Pair;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> RlResult<(usize, Vec<Self::Candidate>)> {
        self.completer.complete(line, pos, ctx)
    }
}

impl Hinter for KataHelper {
    type Hint = String;

    fn hint(&self, _line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<Self::Hint> {
        // Sem sugestões de histórico — o rustyline 14 exibe a sugestão
        // inline com cor ANSI, o que confunde o cálculo de largura e
        // causa eco duplo em alguns terminais.
        None
    }
}

impl Validator for KataHelper {
    fn validate(
        &self,
        _ctx: &mut rustyline::validate::ValidationContext,
    ) -> rustyline::Result<rustyline::validate::ValidationResult> {
        Ok(rustyline::validate::ValidationResult::Valid(None))
    }

    fn validate_while_typing(&self) -> bool {
        false
    }
}

impl Highlighter for KataHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        highlight_line(line)
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        // "kata> " em bold green, "   ... " em gray
        if prompt.ends_with("... ") {
            Owned(format!("{BRIGHT_BLACK}{prompt}{RESET}"))
        } else {
            Owned(format!("{BOLD_GREEN}{prompt}{RESET}"))
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned(format!("{BRIGHT_BLACK}{hint}{RESET}"))
    }

    fn highlight_candidate<'c>(
        &self,
        candidate: &'c str,
        _completion: CompletionType,
    ) -> Cow<'c, str> {
        Owned(format!("{BRIGHT_BLUE}{candidate}{RESET}"))
    }

    fn highlight_char(&self, _line: &str, _pos: usize, kind: CmdKind) -> bool {
        // Re-highlight em edições (Other) e refresh forçado (ForcedRefresh).
        // Não re-highlight em movimento de cursor (MoveCursor).
        //
        // O rustyline 15 mudou a API de highlight_char para distinguir
        // edição de movimento de cursor (PR #812, issue #332). Isto
        // resolve o eco duplo: o refresh só acontece quando o texto muda,
        // e o clear_old_rows limpa corretamente antes de re-renderizar.
        matches!(kind, CmdKind::Other | CmdKind::ForcedRefresh)
    }
}

// Helper é auto-implementado quando os 4 traits estão implementados.
impl rustyline::Helper for KataHelper {}
