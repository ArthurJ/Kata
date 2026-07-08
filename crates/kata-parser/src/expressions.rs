//! Expressions — atoms, application, let, paren, type ascription.

use kata_ast::{Expr, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    /// Determine if the current token can start an expression.
    pub(crate) fn can_start_expr(&self) -> bool {
        matches!(
            self.peek(),
            Token::IntLit(_)
                | Token::FloatLit(_)
                | Token::TextLit(_)
                | Token::Ident(_)
                | Token::LParen
                | Token::Let
                | Token::Lambda
                | Token::Match
        )
    }

    pub(crate) fn parse_let(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Let, "`let`")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("binding name after `let`")),
        };
        self.expect(&Token::BindAssign, "`:=`")?;
        let value = parse_expr(self)?;
        // Cover span
        let span = start.cover(value.span);
        Ok(Spanned::new(
            Expr::Let {
                name,
                value: Box::new(value),
            },
            span,
        ))
    }

    /// Parse a single expression atom (no application).
    pub(crate) fn parse_expr_atom(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        match self.peek().clone() {
            Token::IntLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::IntLit { text: s }, start))
            }
            Token::FloatLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::FloatLit { text: s }, start))
            }
            Token::TextLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::TextLit { text: s }, start))
            }
            Token::Let => self.parse_let(),
            Token::Lambda => self.parse_lambda(),
            Token::Match => self.parse_match(),
            Token::LParen => self.parse_paren_expr(),
            Token::Ident(name) => {
                self.advance();
                // `_` em posição de expressão → Hole (currying).
                // Em posição de pattern, o parser produz Pattern::Wildcard
                // (disambiguação no parser, não no typeck).
                if name == "_" {
                    return Ok(Spanned::new(Expr::Hole, start));
                }
                // Check for VariantQual: Ident :: Ident
                // But `::` is also TypeAscription (expr::Type).
                // Disambiguation: if the Ident after `::` is a known type name vs variant...
                // The parser can't know. The PRD says VariantQual is `Enum::Variant`.
                // TypeAscription is `expr::Type`.
                // Heuristic: `Ident::Ident` where both are capitalized → VariantQual
                // Actually, the PRD says "The parser doesn't know if Boolean is type or
                // module — produces VariantQual and typeck resolves."
                // But `3.14::Rational` is TypeAscription(FloatLit, Named(Rational)).
                // So: if left side is a literal → TypeAscription
                // If left side is an Ident and we see `::` followed by Ident → VariantQual
                // If left side is an Ident and we see `::` followed by something that's a
                // type expression (not a simple Ident that could be a variant) → ambiguous
                //
                // For Fio 1: the only use of `::` with an Ident on the left is VariantQual.
                // TypeAscription with `::` is only for literals (`3.14::Rational`).
                // So: Ident :: Ident → VariantQual
                //     Literal :: Type → TypeAscription
                if matches!(self.peek(), Token::DoubleColon) {
                    // Ident :: Ident → VariantQual
                    if let Some(next) = self.tokens.get(self.pos + 1) {
                        if let Token::Ident(variant) = &next.token {
                            let variant = variant.clone();
                            self.advance(); // consume ::
                            self.advance(); // consume variant Ident
                            let span = start.cover(self.tokens[self.pos - 1].span);
                            return Ok(Spanned::new(
                                Expr::VariantQual {
                                    enum_name: name,
                                    variant,
                                },
                                span,
                            ));
                        }
                    }
                }
                Ok(Spanned::new(Expr::Ident { name }, start))
            }
            _ => Err(self.error("expression")),
        }
    }

    /// Parse parenthesized expression: `()`, `(expr)`, `(a, b, c)`, `(a,)`
    pub(crate) fn parse_paren_expr(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::LParen, "`(`")?;

        // `()` = Unit
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            return Ok(Spanned::new(Expr::Unit, start));
        }

        // Parse first expression
        let first = parse_expr(self)?;

        // No comma → grouping
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            let span = start.cover(first.span);
            return Ok(Spanned::new(
                Expr::Grouping {
                    inner: Box::new(first),
                },
                span,
            ));
        }

        // Comma present → tuple
        let mut elements = vec![first];
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            // Trailing comma: `(a,)` → tuple of 1
            if matches!(self.peek(), Token::RParen) {
                break;
            }
            elements.push(parse_expr(self)?);
        }

        self.expect(&Token::RParen, "`)`")?;
        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(Expr::Tuple { elements }, span))
    }

    /// Parse an expression with greedy application.
    pub(crate) fn parse_expr_post_ascription(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let atom = self.parse_expr_atom()?;

        // Check for TypeAscription: expr::Type
        if matches!(self.peek(), Token::DoubleColon) {
            self.advance(); // consume ::
            let ty = self.parse_type_expr()?;
            let span = atom.span.cover(ty.span);
            return Ok(Spanned::new(
                Expr::TypeAscription {
                    expr: Box::new(atom),
                    ty,
                },
                span,
            ));
        }

        Ok(atom)
    }

    /// Parse an atom or ascription — used for arguments in Apply.
    /// Arguments don't greedily consume more arguments themselves.
    pub(crate) fn parse_expr_atom_or_ascription(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        self.parse_expr_post_ascription()
    }

    /// Parse `lambda <patterns>: <body>` — lambda anônimo (cláusula única).
    ///
    /// Fase 3: body é expressão única após `:` (sem guards, sem with).
    /// Fase 6: body pode ser bloco indentado com guard clauses + with.
    pub(crate) fn parse_lambda(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Lambda, "`lambda`")?;

        // Parse patterns (1 ou mais, separados por espaço)
        let patterns = self.parse_patterns()?;

        // Expect `:`
        self.expect(&Token::Colon, "`:` após patterns do lambda")?;

        // Fase 3: body é uma expressão única (sem guards).
        // Fase 6: se há INDENT após `:`, é bloco com guards + with.
        let body = if matches!(self.peek(), Token::Indent) {
            // Bloco indentado — Fase 6 implementará guards + with.
            // Por agora, parsear como bloco de guard clauses.
            self.parse_lambda_body_block(start, patterns.len())?
        } else {
            // Expressão única na mesma linha
            let body_expr = parse_expr(self)?;
            body_expr
        };

        let span = start.cover(body.span);
        Ok(Spanned::new(
            Expr::Lambda {
                patterns,
                body: Box::new(body),
                guards: Vec::new(),
                with_bindings: Vec::new(),
            },
            span,
        ))
    }

    /// Parse o bloco indentado de guards dentro de um lambda body.
    /// Fase 6: implementação completa com guards + with.
    /// Por agora (Fase 3), não deveria ser chamado — todo!().
    fn parse_lambda_body_block(
        &mut self,
        _start: kata_ast::Span,
        _num_patterns: usize,
    ) -> Result<Spanned<Expr>, FrontendError> {
        // Fase 6 implementará: INDENT guard_clause+ (with with_binding+)? DEDENT
        // Por agora, se chegamos aqui, é erro.
        todo!("Fase 6: parse_lambda_body_block com guards + with")
    }
}

/// Parse an expression with greedy application.
/// Free function — called from declarations and expressions.
pub(crate) fn parse_expr(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
    // Parse the callee/first expression
    let callee = parser.parse_expr_post_ascription()?;

    // Greedily collect arguments
    let mut args = Vec::new();
    while parser.can_start_expr() {
        args.push(parser.parse_expr_atom_or_ascription()?);
    }

    if args.is_empty() {
        Ok(callee)
    } else {
        let span = callee.span.cover(args.last().expect("non-empty args").span);
        Ok(Spanned::new(
            Expr::Apply {
                callee: Box::new(callee),
                args,
            },
            span,
        ))
    }
}
