//! Parser recursive-descent, prefix-only (sem Pratt parsing).
//!
//! Consome `Vec<TokenWithSpan>` do lexer e produz `Module` (AST plana).
//! A notação prefixa elimina precedência de operadores — `+`, `soma`,
//! `fatorial` são todos identificadores tratados identicamente.
//!
//! Aplicação é greedy: `f a b c` vira um único `Apply { callee: f, args: [a, b, c] }`.

mod _match;
mod _select;
mod action_decl;
mod casing;
mod declarations;
mod directive_decl;
mod directives;
mod expr_apply;
mod expr_containers;
mod expressions;
mod imports;
mod interface_decl;
mod lambda;
mod patterns;
mod sig;
mod type_decls;
mod types;

use kata_ast::{Module, Span, Token, TokenWithSpan};
use kata_diagnostics::{FrontendError, MietteSpan};

pub(crate) use casing::{CasingPattern, is_snake_case, validate_casing};

// ────────────────────────────────────────────────────────────────
// Parser state
// ────────────────────────────────────────────────────────────────

pub(crate) struct Parser {
    pub(crate) tokens: Vec<TokenWithSpan>,
    pub(crate) pos: usize,
    /// Flag: true quando o parser está dentro do body de uma Action.
    /// `var` e `return` só são aceitos quando esta flag é true.
    /// Fora de Action, produzem erro de parser.
    pub(crate) in_action_body: bool,
    /// Tabela de aridades para parsing arity-aware.
    /// `None` = modo greedy (comportamento atual).
    /// `Some(map)` = o parser coleta exatamente `map[name]` args posicionais
    /// para `Apply` onde o callee é `Ident(name)`.
    pub(crate) arities: Option<std::collections::HashMap<String, usize>>,
    /// Modo REPL: quando true, `let` no top level é aceito como EntryExpr
    /// (PRD §2.5 — o REPL não é top-level de módulo).
    pub(crate) repl_mode: bool,
    /// Profundidade de aninhamento de expressão atual.
    /// Incrementada em `parse_expr` a cada nível de recursão.
    /// Limita aninhamento para evitar stack overflow no recursive descent.
    pub(crate) depth: usize,
}

/// 256 níveis de aninhamento de estruturas (listas, parênteses, etc.).
/// `parse_expr` incrementa depth a cada chamada; a chamada top-level
/// (não-aninhada) é depth 0. Com `> MAX_EXPR_DEPTH`, 256 níveis de `[`
/// geram depth 256 (aceito) e 257 níveis geram depth 257 (rejeitado).
pub(crate) const MAX_EXPR_DEPTH: usize = 256;

impl Parser {
    pub(crate) fn new(tokens: Vec<TokenWithSpan>) -> Self {
        Parser {
            tokens,
            pos: 0,
            in_action_body: false,
            arities: None,
            repl_mode: false,
            depth: 0,
        }
    }

    /// Constrói um Parser em modo arity-aware.
    pub(crate) fn new_with_arities(
        tokens: Vec<TokenWithSpan>,
        arities: std::collections::HashMap<String, usize>,
    ) -> Self {
        Parser {
            tokens,
            pos: 0,
            in_action_body: false,
            arities: Some(arities),
            repl_mode: false,
            depth: 0,
        }
    }

    // ── Token access helpers ──────────────────────────────────────

    pub(crate) fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    pub(crate) fn peek_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    pub(crate) fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    /// Advance and return the span of the consumed token.
    pub(crate) fn advance(&mut self) -> Span {
        let span = self.tokens[self.pos].span;
        self.pos += 1;
        span
    }

    pub(crate) fn error(&self, expected: &str) -> FrontendError {
        let found = self.peek().clone();
        FrontendError::UnexpectedToken {
            expected: expected.to_string(),
            found: found.to_string(),
            span: MietteSpan(self.peek_span()),
        }
    }

    /// Consume a token matching the given predicate, returning its span.
    pub(crate) fn expect(&mut self, expected: &Token, label: &str) -> Result<Span, FrontendError> {
        if self.peek() == expected {
            Ok(self.advance())
        } else {
            Err(self.error(label))
        }
    }

    /// Valida `name` contra `expected`. Se inválido, retorna
    /// `FrontendError::InvalidCasing` com o span do nome.
    ///
    /// Nomes contendo `__` (duplo underscore em qualquer posição) são
    /// rejeitados (`ReservedName`) — são reservados para símbolos gerados
    /// pelo compilador e valores injetados pelo runtime (ex: `__stdin__`,
    /// `__stdout__`, `__stderr__`, que o usuário referencia mas não cria).
    ///
    /// A validação de casing só se aplica a nomes alfabéticos (não símbolos
    /// como `+`, `-`, `*`). Nomes começando com `_` são válidos em snake_case.
    pub(crate) fn validate_name(
        &self,
        name: &str,
        expected: CasingPattern,
        span: Span,
    ) -> Result<(), FrontendError> {
        if name.contains("__") {
            return Err(FrontendError::ReservedName {
                name: name.to_string(),
                span: span.into(),
            });
        }
        // Casing só para nomes alfabéticos (não símbolos como +, -, *).
        if name.chars().next().is_some_and(|c| c.is_alphabetic()) {
            validate_casing(name, expected, span)?;
        }
        Ok(())
    }
}

// ────────────────────────────────────────────────────────────────
// Public API
// ────────────────────────────────────────────────────────────────

/// Parse a token stream into a `Module`.
///
/// This is the main entry point. It consumes the tokens produced by the lexer
/// and produces a `Module` (list of `Spanned<Item>`).
pub fn parse(tokens: Vec<TokenWithSpan>) -> Result<Module, FrontendError> {
    let mut parser = Parser::new(tokens);
    parser.parse_module()
}

/// Parse a token stream with arity-aware application parsing.
///
/// When the parser encounters `Apply` where the callee is `Ident(name)` and
/// `arities[name]` exists, it collects exactly N positional arguments (each
/// parsed via `parse_apply` for sub-application), then errors if excess tokens
/// follow without a `StmtSep`. When `arities` has no entry for a name, falls
/// back to greedy atom collection (current behavior).
pub fn parse_with_arity(
    tokens: Vec<TokenWithSpan>,
    arities: std::collections::HashMap<String, usize>,
) -> Result<Module, FrontendError> {
    let mut parser = Parser::new_with_arities(tokens, arities);
    parser.parse_module()
}

/// Parse with arity-aware application **and** error recovery.
///
/// Combina `parse_with_arity` (arity-aware) com `parse_with_recovery`
/// (error recovery de top-level items). Quando um item falha, registra
/// o erro e skipa tokens até o próximo `StmtSep` ou `Eof`, então continua
/// parseando o próximo item. Retorna sempre `Ok` com:
/// - `Module` contendo os items parseados com sucesso (pode ser vazio)
/// - `Vec<FrontendError>` com os erros encontrados (vazio se tudo ok)
///
/// Usado pelo pipeline do driver para reportar múltiplos erros de parse
/// em uma única passada, em vez de abortar no primeiro.
pub fn parse_with_arity_recovery(
    tokens: Vec<TokenWithSpan>,
    arities: std::collections::HashMap<String, usize>,
) -> (Module, Vec<FrontendError>) {
    let mut parser = Parser::new_with_arities(tokens, arities);
    parser.parse_module_with_recovery()
}

/// Parse apenas declarações (Sigs, implements, data, enum, action defs,
/// imports, exports, alias, interface). Entry exprs são **skipadas** —
/// os tokens são consumidos até o próximo `StmtSep` ou `Eof` sem produzir
/// AST.
///
/// Usado pelo Pass 1 do ciclo de dois passes: extrai assinaturas
/// sem precisar de aridades (declarações não dependem de arity-aware
/// parsing).
pub fn parse_decls_only(tokens: Vec<TokenWithSpan>) -> Result<Module, FrontendError> {
    let mut parser = Parser::new(tokens);
    parser.parse_module_decls_only()
}

/// Parse a token stream in REPL mode.
///
/// When `repl_mode` is true, `let` at the top level is accepted as an
/// `EntryExpr` (PRD §2.5 — the REPL is not module top-level).
pub fn parse_repl(tokens: Vec<TokenWithSpan>) -> Result<Module, FrontendError> {
    let mut parser = Parser::new(tokens);
    parser.repl_mode = true;
    parser.parse_module()
}

/// Parse a token stream in REPL mode with arity-aware application parsing.
pub fn parse_repl_with_arity(
    tokens: Vec<TokenWithSpan>,
    arities: std::collections::HashMap<String, usize>,
) -> Result<Module, FrontendError> {
    let mut parser = Parser::new_with_arities(tokens, arities);
    parser.repl_mode = true;
    parser.parse_module()
}

/// Parse only declarations in REPL mode.
pub fn parse_repl_decls_only(tokens: Vec<TokenWithSpan>) -> Result<Module, FrontendError> {
    let mut parser = Parser::new(tokens);
    parser.repl_mode = true;
    parser.parse_module_decls_only()
}

/// Scan de tokens para extrair aridades de lambdas em bindings `let` do top level.
///
/// Procura o padrão `let IDENT := lambda <params> :` e conta os params
/// para produzir um mapa de aridades. Não constrói AST — é um scan linear
/// O(n) sobre os tokens. Casos não-lambda (`let f := compose g h`,
/// `let f := if ...`) são skipados silenciosamente.
///
/// Usado no Pass 0 (antes do Pass 1 de parse_decls_only) para que funções
/// definidas via `let f := lambda x: ...` também tenham arity-aware parsing.
/// Signatures do Pass 1 sobrescrevem — a aridade padrão vem sempre da
/// signature declarada no top level. Lambdas com mesmo nome são overloads
/// non-default (só acessíveis via dict dispatch).
///
/// Só extrai o caso direto: `let IDENT := lambda ...`. Lambdas indiretas
/// (dentro de `if`, `match`, composição, etc.) não são extraíveis sem
/// inferência de tipos e ficam em greedy (fallback seguro).
pub fn scan_lambdas(tokens: &[TokenWithSpan]) -> std::collections::HashMap<String, usize> {
    let mut arities = std::collections::HashMap::new();
    let mut i = 0;

    while i < tokens.len() {
        // Procura `let` no top level. Como entry exprs são planas no top
        // level e `let` é keyword, qualquer `Token::Let` é um binding.
        if tokens[i].token != Token::Let {
            i += 1;
            continue;
        }
        i += 1;

        // `let IDENT := lambda ...`
        // Verifica o padrão sem let-chains (requer Rust 2024).
        if i + 3 >= tokens.len() {
            // Não há tokens suficientes — skipa até StmtSep/Eof
            while i < tokens.len()
                && tokens[i].token != Token::StmtSep
                && tokens[i].token != Token::Eof
            {
                i += 1;
            }
            continue;
        }

        // Extrai nome e verifica se é `IDENT := lambda`
        let name = match &tokens[i].token {
            Token::Ident(n) => n.clone(),
            _ => {
                // `let` sem Ident — skipa até StmtSep/Eof
                while i < tokens.len()
                    && tokens[i].token != Token::StmtSep
                    && tokens[i].token != Token::Eof
                {
                    i += 1;
                }
                continue;
            }
        };

        if tokens[i + 1].token != Token::BindAssign || tokens[i + 2].token != Token::Lambda {
            // `let IDENT := <não-lambda>` — skipa até StmtSep/Eof
            while i < tokens.len()
                && tokens[i].token != Token::StmtSep
                && tokens[i].token != Token::Eof
            {
                i += 1;
            }
            continue;
        }

        // Contar params: tokens entre `lambda` e `:` (Colon).
        // Params podem ser:
        //   - Ident (pattern simples): `x`, `y`
        //   - Ident :: Type (TypedIdent): `x::Int` → DoubleColon + type tokens
        //   - (p1, p2) (Tuple pattern): LParen ... RParen
        //   - [h : t] (Cons pattern): LBracket ... RBracket
        //   - _ (Wildcard): Ident("_")
        //
        // Em vez de parsear patterns propriamente, contamos
        // "inícios de pattern" entre `lambda` e `:`. Um início de
        // pattern é: Ident, IntLit, FloatLit, TextLit, LParen, LBracket.
        // Mas precisamos ignorar Idents que são parte de type annotations
        // (após DoubleColon). A estratégia: contar tokens que
        // can_start_pattern quando estamos "no nível zero" (depth=0
        // em parênteses/colchetes) e não estamos dentro de uma
        // type annotation.
        i += 3; // pula Ident, BindAssign, Lambda
        let mut count = 0usize;
        let mut depth = 0i32;
        let mut in_type_ann = false;

        while i < tokens.len() {
            match &tokens[i].token {
                Token::Colon => break,
                Token::LParen | Token::LBracket | Token::LBrace => {
                    depth += 1;
                    in_type_ann = false;
                }
                Token::RParen | Token::RBracket | Token::RBrace => {
                    depth -= 1;
                    in_type_ann = false;
                }
                Token::DoubleColon if depth == 0 => {
                    in_type_ann = true;
                }
                Token::StmtSep | Token::Eof | Token::Indent => break,
                Token::Comma | Token::Dot | Token::DotDot | Token::DotDotEq => {
                    // Separadores dentro de patterns — ignorar
                }
                Token::Ident(_) | Token::IntLit(_) | Token::FloatLit(_) | Token::TextLit(_) => {
                    if depth == 0 && !in_type_ann {
                        count += 1;
                    }
                    in_type_ann = false;
                }
                _ => {
                    // Outros tokens (FatArrow, ThinArrow, etc.) —
                    // resetam in_type_ann mas não contam como pattern
                    in_type_ann = false;
                }
            }
            i += 1;
        }

        if count > 0 {
            arities.insert(name, count);
        }
        // Continua o scan — i já está no `:` ou beyond
    }

    arities
}

/// Parse with error recovery — acumula erros de top-level items.
///
/// Diferente de `parse`, não aborta no primeiro erro. Quando um item falha,
/// registra o erro e skipa tokens até o próximo `StmtSep` ou `Eof`, então
/// continua parseando o próximo item. Retorna sempre `Ok` com:
/// - `Module` contendo os items parseados com sucesso (pode ser vazio)
/// - `Vec<FrontendError>` com os erros encontrados (vazio se tudo ok)
///
/// Usado pelo LSP para dar múltiplos diagnósticos em um único pass,
/// mantendo os items válidos para hover mesmo quando há erros.
pub fn parse_with_recovery(tokens: Vec<TokenWithSpan>) -> (Module, Vec<FrontendError>) {
    let mut parser = Parser::new(tokens);
    parser.parse_module_with_recovery()
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
