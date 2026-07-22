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
    /// - `[h : t]` → Cons (stub )
    /// - `[]` → Cons Nil (stub )
    pub(crate) fn parse_pattern(&mut self) -> Result<Spanned<Pattern>, FrontendError> {
        self.parse_pattern_inner(false)
    }

    /// Parse um pattern de match arm — difere de `parse_pattern` por tratar
    /// `Ident` seguido de sub-pattern como variante desqualificada com payload.
    ///
    /// Em match arms, cada braço tem exatamente um pattern, então `Ok v` só
    /// pode significar "variante Ok com payload v" — não há ambiguidade com
    /// múltiplos argumentos (como em lambda clauses).
    ///
    /// `Ident` sozinho (sem sub-pattern) continua como `Pattern::Ident` —
    /// o typeck resolve variantes unitárias via EnumRegistry.
    pub(crate) fn parse_match_pattern(&mut self) -> Result<Spanned<Pattern>, FrontendError> {
        self.parse_pattern_inner(true)
    }

    fn parse_pattern_inner(
        &mut self,
        allow_unqualified_variant: bool,
    ) -> Result<Spanned<Pattern>, FrontendError> {
        let start = self.peek_span();
        match self.peek().clone() {
            // `_` → Wildcard
            Token::Ident(s) if s == "_" => {
                self.advance();
                Ok(Spanned::new(Pattern::Wildcard, start))
            }
            // `Enum::Variant` → Variant qualificado (possivelmente com sub-pattern)
            Token::Ident(name) => {
                self.advance();
                if matches!(self.peek(), Token::DoubleColon)
                    && let Some(next) = self.tokens.get(self.pos + 1)
                    && let Token::Ident(variant) = &next.token
                {
                    let variant = variant.clone();
                    self.advance(); // consume ::
                    self.advance(); // consume variant

                    // Verificar se há um sub-pattern após a variante.
                    // `Result::Ok v` → Variant com payload sub-pattern.
                    // `Result::Ok(v)` → Variant com payload sub-pattern (entre parênteses).
                    // `Boolean::True` (sem sub-pattern) → Variant sem payload.
                    let payload = if self.can_start_pattern() {
                        // Sub-pattern sem parênteses: `Result::Ok v`
                        Some(vec![self.parse_pattern()?])
                    } else if matches!(self.peek(), Token::LParen) {
                        // Sub-patterns entre parênteses: `Result::Ok(v)`
                        self.advance(); // consume (
                        let mut sub_pats = Vec::new();
                        if !matches!(self.peek(), Token::RParen) {
                            sub_pats.push(self.parse_pattern()?);
                            while matches!(self.peek(), Token::Comma) {
                                self.advance();
                                if matches!(self.peek(), Token::RParen) {
                                    break;
                                }
                                sub_pats.push(self.parse_pattern()?);
                            }
                        }
                        self.expect(&Token::RParen, "`)` após sub-pattern do variant")?;
                        Some(sub_pats)
                    } else {
                        None
                    };

                    let end_span = self
                        .tokens
                        .get(self.pos - 1)
                        .map(|t| t.span)
                        .unwrap_or(start);
                    let span = start.cover(end_span);
                    return Ok(Spanned::new(
                        Pattern::Variant {
                            enum_name: name,
                            variant,
                            payload,
                        },
                        span,
                    ));
                }
                // Ident sem `::` — pode ser binding, variante unitária, ou
                // variante desqualificada com payload.
                //
                // Em match arms (allow_unqualified_variant=true), se o próximo
                // token pode iniciar um pattern, tratamos como variante
                // desqualificada com payload: `Ok v` → Variant{enum_name:"", variant:"Ok", payload:[v]}.
                // O typeck resolve enum_name via EnumRegistry do scrutinee.
                //
                // Sem sub-pattern following, continua como Pattern::Ident —
                // o typeck resolve variantes unitárias (True, False, None) via EnumRegistry.
                if allow_unqualified_variant && self.can_start_pattern() {
                    let sub_pat = self.parse_pattern()?;
                    let end_span = sub_pat.span;
                    let span = start.cover(end_span);
                    return Ok(Spanned::new(
                        Pattern::Variant {
                            enum_name: String::new(),
                            variant: name,
                            payload: Some(vec![sub_pat]),
                        },
                        span,
                    ));
                }
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
            // `[]` ou `[h : t]` → Cons pattern (stub )
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

    /// `[h : t]` → Cons pattern, `[]` → Nil pattern.
    fn parse_cons_pattern(
        &mut self,
        start: kata_ast::Span,
    ) -> Result<Spanned<Pattern>, FrontendError> {
        self.expect(&Token::LBracket, "`[`")?;

        // `[]` → Nil pattern (lista vazia). Codegen testa val == 0.
        if matches!(self.peek(), Token::RBracket) {
            self.advance();
            return Ok(Spanned::new(Pattern::Nil, start));
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
