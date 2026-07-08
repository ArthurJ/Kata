//! Patterns — parse_pattern para match arms e cláusulas lambda.
//!
//! Patterns são reusados integralmente entre `match` e cláusulas lambda.
//! Disambiguação no parser: `_` em posição de pattern → `Wildcard`.
//! `True` em posição de pattern → `Ident("True")` (typeck resolve via
//! `EnumRegistry` para `Variant` se for variante de enum do scrutinee).

use kata_ast::{Expr, Pattern, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    /// Parse um pattern único.
    ///
    /// Patterns suportados:
    /// - `_` → Wildcard
    /// - `42`, `"texto"`, `3.14` → Literal
    /// - `Ident` → Ident (pode ser variante desqualificada — typeck resolve)
    /// - `Enum::Variant` → Variant (qualificado)
    /// - `(p1, p2, ...)` → Tuple
    /// - `[h : t]` → Cons (stub Fio 8)
    /// - `[]` → Cons Nil (stub Fio 8)
    pub(crate) fn parse_pattern(&mut self) -> Result<Spanned<Pattern>, FrontendError> {
        let start = self.peek_span();
        match self.peek().clone() {
            // `_` → Wildcard
            Token::Ident(s) if s == "_" => {
                self.advance();
                Ok(Spanned::new(Pattern::Wildcard, start))
            }
            // `Enum::Variant` → Variant qualificado
            Token::Ident(name) => {
                self.advance();
                if matches!(self.peek(), Token::DoubleColon)
                    && let Some(next) = self.tokens.get(self.pos + 1)
                    && let Token::Ident(variant) = &next.token
                {
                    let variant = variant.clone();
                    self.advance(); // consume ::
                    self.advance(); // consume variant
                    let span = start.cover(self.tokens[self.pos - 1].span);
                    return Ok(Spanned::new(
                        Pattern::Variant {
                            enum_name: name,
                            variant,
                        },
                        span,
                    ));
                }
                // Ident simples — pode ser binding ou variante desqualificada.
                // O typeck resolve via EnumRegistry.
                Ok(Spanned::new(Pattern::Ident(name), start))
            }
            // Literais → Literal pattern
            Token::IntLit(s) => {
                self.advance();
                Ok(Spanned::new(
                    Pattern::Literal(Spanned::new(Expr::IntLit { text: s }, start)),
                    start,
                ))
            }
            Token::FloatLit(s) => {
                self.advance();
                Ok(Spanned::new(
                    Pattern::Literal(Spanned::new(Expr::FloatLit { text: s }, start)),
                    start,
                ))
            }
            Token::TextLit(s) => {
                self.advance();
                Ok(Spanned::new(
                    Pattern::Literal(Spanned::new(Expr::TextLit { text: s }, start)),
                    start,
                ))
            }
            // `()` → Unit literal pattern
            Token::LParen => self.parse_tuple_pattern(start),
            // `[]` ou `[h : t]` → Cons pattern (stub Fio 8)
            Token::LBracket => self.parse_cons_pattern(start),
            _ => Err(self.error("pattern")),
        }
    }

    /// Parse `(p1, p2, ...)` → Tuple pattern.
    /// `()` → Tuple vazia (ou Unit — typeck decide).
    fn parse_tuple_pattern(
        &mut self,
        start: kata_ast::Span,
    ) -> Result<Spanned<Pattern>, FrontendError> {
        self.expect(&Token::LParen, "`(`")?;

        // `()` → Tuple vazio
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            return Ok(Spanned::new(Pattern::Tuple(Vec::new()), start));
        }

        let mut elements = vec![self.parse_pattern()?];
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            if matches!(self.peek(), Token::RParen) {
                break; // trailing comma
            }
            elements.push(self.parse_pattern()?);
        }

        self.expect(&Token::RParen, "`)`")?;
        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(Pattern::Tuple(elements), span))
    }

    /// Parse `[h : t]` → Cons pattern, `[]` → Cons Nil (stub Fio 8).
    fn parse_cons_pattern(
        &mut self,
        start: kata_ast::Span,
    ) -> Result<Spanned<Pattern>, FrontendError> {
        self.expect(&Token::LBracket, "`[`")?;

        // `[]` → Nil pattern (stub)
        if matches!(self.peek(), Token::RBracket) {
            self.advance();
            // Nil não é um Pattern variant próprio — usamos Cons com Wildcard
            // como stub. O typeck rejeita com erro "List patterns são Fio 8".
            // Representação: Cons { head: Wildcard, tail: Wildcard } marcado
            // como nil. Como não há variant Nil, simplificamos: o typeck
            // rejeita qualquer Cons pattern em Fio 2.
            return Ok(Spanned::new(
                Pattern::Cons {
                    head: Box::new(Spanned::new(Pattern::Wildcard, start)),
                    tail: Box::new(Spanned::new(Pattern::Wildcard, start)),
                },
                start,
            ));
        }

        // `[h : t]` → Cons
        let head = self.parse_pattern()?;
        self.expect(&Token::Colon, "`:` (cons pattern)")?;
        let tail = self.parse_pattern()?;
        self.expect(&Token::RBracket, "`]`")?;
        let span = start.cover(tail.span);
        Ok(Spanned::new(
            Pattern::Cons {
                head: Box::new(head),
                tail: Box::new(tail),
            },
            span,
        ))
    }

    /// Parse múltiplos patterns separados por espaço (argumentos de lambda).
    pub(crate) fn parse_patterns(&mut self) -> Result<Vec<Spanned<Pattern>>, FrontendError> {
        let mut patterns = Vec::new();
        // Pelo menos 1 pattern
        patterns.push(self.parse_pattern()?);
        while self.can_start_pattern() {
            patterns.push(self.parse_pattern()?);
        }
        Ok(patterns)
    }

    /// Verifica se o token atual pode iniciar um pattern.
    pub(crate) fn can_start_pattern(&self) -> bool {
        matches!(
            self.peek(),
            Token::Ident(_)
                | Token::IntLit(_)
                | Token::FloatLit(_)
                | Token::TextLit(_)
                | Token::LParen
                | Token::LBracket
        )
    }
}
