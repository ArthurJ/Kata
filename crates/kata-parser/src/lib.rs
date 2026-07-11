//! Parser recursive-descent, prefix-only (sem Pratt parsing).
//!
//! Consome `Vec<TokenWithSpan>` do lexer e produz `Module` (AST plana).
//! A notação prefixa elimina precedência de operadores — `+`, `soma`,
//! `fatorial` são todos identificadores tratados identicamente.
//!
//! Aplicação é greedy: `f a b c` vira um único `Apply { callee: f, args: [a, b, c] }`.

mod _match;
mod declarations;
mod directives;
mod expressions;
mod lambda;
mod patterns;
mod types;

use kata_ast::{Module, Span, Token, TokenWithSpan};
use kata_diagnostics::{FrontendError, MietteSpan};

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
}

impl Parser {
    pub(crate) fn new(tokens: Vec<TokenWithSpan>) -> Self {
        Parser {
            tokens,
            pos: 0,
            in_action_body: false,
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
                Expr::VariantQual { enum_name, variant } => {
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
                assert_eq!(
                    directives[0].args,
                    vec![DirectiveArg::Str("kata_rt_bi_add".into())]
                );
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
                assert_eq!(directives[0].args, vec![DirectiveArg::Int(0)]);
            }
            other => panic!("expected Sig, got {other:?}"),
        }
    }
}
