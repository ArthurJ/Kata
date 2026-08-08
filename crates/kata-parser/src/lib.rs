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
}

impl Parser {
    pub(crate) fn new(tokens: Vec<TokenWithSpan>) -> Self {
        Parser {
            tokens,
            pos: 0,
            in_action_body: false,
            arities: None,
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

    /// Valida casing de `name` contra `expected`. Se inválido, retorna
    /// `FrontendError::InvalidCasing` com o span do nome.
    pub(crate) fn validate_name(
        &self,
        name: &str,
        expected: CasingPattern,
        span: Span,
    ) -> Result<(), FrontendError> {
        validate_casing(name, expected, span)
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

/// Parse apenas declarações (Sigs, implements, data, enum, action defs,
/// imports, exports, alias, interface). Entry exprs são **skipadas** —
/// os tokens são consumidos até o próximo `StmtSep` ou `Eof` sem produzir
/// AST.
///
/// Usado pelo Pass 1 do ciclo de dois passes (Fase 4): extrai assinaturas
/// sem precisar de aridades (declarações não dependem de arity-aware
/// parsing).
pub fn parse_decls_only(tokens: Vec<TokenWithSpan>) -> Result<Module, FrontendError> {
    let mut parser = Parser::new(tokens);
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
mod tests {
    use super::*;
    use kata_ast::{DirectiveArg, Expr, Item, TypeExpr};
    use kata_lexer::lex;

    fn parse_src(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        parse(tokens).unwrap()
    }

    fn first_item(m: &Module) -> &Item {
        &m.items.first().expect("at least one item").node
    }

    #[test]
    fn apply_plus_1_2() {
        let m = parse_src("+ 1 2");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::Apply { callee, args } => {
                    assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                    assert_eq!(args.len(), 2);
                    assert_eq!(args[0].node, Expr::IntLit { text: "1".into() });
                    assert_eq!(args[1].node, Expr::IntLit { text: "2".into() });
                }
                other => panic!("expected Apply, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn let_binding() {
        let m = parse_src("let x := 42");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::Let { name, value } => {
                    assert_eq!(name, "x");
                    assert_eq!(value.node, Expr::IntLit { text: "42".into() });
                }
                other => panic!("expected Let, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn type_ascription_rational() {
        let m = parse_src("3.14::Rational");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::TypeAscription { expr, ty } => {
                    assert_eq!(
                        expr.node,
                        Expr::FloatLit {
                            text: "3.14".into()
                        }
                    );
                    assert_eq!(ty.node, TypeExpr::Named("Rational".into()));
                }
                other => panic!("expected TypeAscription, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn tuple_three_elements() {
        let m = parse_src("(1, 2, 3)");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::Tuple { elements } => {
                    assert_eq!(elements.len(), 3);
                    assert_eq!(elements[0].node, Expr::IntLit { text: "1".into() });
                    assert_eq!(elements[1].node, Expr::IntLit { text: "2".into() });
                    assert_eq!(elements[2].node, Expr::IntLit { text: "3".into() });
                }
                other => panic!("expected Tuple, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn grouping_single() {
        let m = parse_src("(42)");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::Grouping { inner } => {
                    assert_eq!(inner.node, Expr::IntLit { text: "42".into() });
                }
                other => panic!("expected Grouping, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn unit_lit() {
        let m = parse_src("()");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => assert_eq!(e.node, Expr::Unit),
            other => panic!("expected EntryExpr(Unit), got {other:?}"),
        }
    }

    #[test]
    fn variant_qual() {
        let m = parse_src("Boolean::True");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::VariantQual {
                    enum_name, variant, ..
                } => {
                    assert_eq!(enum_name, "Boolean");
                    assert_eq!(variant, "True");
                }
                other => panic!("expected VariantQual, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn data_decl_empty() {
        let m = parse_src("data Int ()");
        let item = first_item(&m);
        match item {
            Item::DataDecl {
                name,
                fields,
                directives,
                ..
            } => {
                assert_eq!(name, "Int");
                assert!(fields.is_empty());
                assert!(directives.is_empty());
            }
            other => panic!("expected DataDecl, got {other:?}"),
        }
    }

    #[test]
    fn enum_decl_variants() {
        let m = parse_src("enum Boolean\n    True\n    False");
        let item = first_item(&m);
        match item {
            Item::EnumDecl {
                name,
                variants,
                directives,
            } => {
                assert_eq!(name, "Boolean");
                assert_eq!(variants.len(), 2);
                assert_eq!(variants[0].name, "True");
                assert_eq!(variants[1].name, "False");
                assert!(directives.is_empty());
            }
            other => panic!("expected EnumDecl, got {other:?}"),
        }
    }

    #[test]
    fn sig_simple() {
        let m = parse_src("+ :: Int Int => Int");
        let item = first_item(&m);
        match item {
            Item::Sig {
                name,
                params,
                ret,
                directives,
                body,
                ..
            } => {
                assert_eq!(name, "+");
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
                assert_eq!(params[1].node, TypeExpr::Named("Int".into()));
                assert_eq!(ret.node, TypeExpr::Named("Int".into()));
                assert!(directives.is_empty());
                assert!(body.is_none());
            }
            other => panic!("expected Sig, got {other:?}"),
        }
    }

    #[test]
    fn directive_ffi() {
        let m = parse_src("@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int");
        let item = first_item(&m);
        match item {
            Item::Sig {
                name, directives, ..
            } => {
                assert_eq!(name, "+");
                assert_eq!(directives.len(), 1);
                assert_eq!(directives[0].name, "ffi");
                assert_eq!(directives[0].args.len(), 1);
                match &directives[0].args[0] {
                    DirectiveArg::Expr(e) => {
                        assert_eq!(
                            e.node,
                            Expr::TextLit {
                                text: "kata_rt_bi_add".into()
                            }
                        );
                    }
                    other => panic!("expected Expr arg, got {other:?}"),
                }
            }
            other => panic!("expected Sig with directive, got {other:?}"),
        }
    }

    #[test]
    fn directive_associative_int() {
        let m = parse_src("@associative(0)\n+ :: Int Int => Int");
        let item = first_item(&m);
        match item {
            Item::Sig { directives, .. } => {
                assert_eq!(directives.len(), 1);
                assert_eq!(directives[0].name, "associative");
                assert_eq!(directives[0].args.len(), 1);
                match &directives[0].args[0] {
                    DirectiveArg::Expr(e) => {
                        assert_eq!(e.node, Expr::IntLit { text: "0".into() });
                    }
                    other => panic!("expected Expr arg, got {other:?}"),
                }
            }
            other => panic!("expected Sig, got {other:?}"),
        }
    }
}
