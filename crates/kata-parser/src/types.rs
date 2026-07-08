//! Type expressions — Named, Unit, Grouping, Func.

use kata_ast::{Spanned, Token, TypeExpr};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    pub(crate) fn parse_type_expr(&mut self) -> Result<Spanned<TypeExpr>, FrontendError> {
        let start = self.peek_span();
        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance();
                Ok(Spanned::new(TypeExpr::Named(name), start))
            }
            Token::LParen => {
                self.advance(); // consume (
                // `()` = Unit type
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    return Ok(Spanned::new(TypeExpr::Unit, start));
                }
                let first = self.parse_type_expr()?;

                // No comma → either grouping `(T)` or function type `(T) -> U`
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    // Check for `->` (function type with single param)
                    if matches!(self.peek(), Token::ThinArrow) {
                        self.advance();
                        let ret = self.parse_type_expr()?;
                        let span = start.cover(ret.span);
                        return Ok(Spanned::new(
                            TypeExpr::Func {
                                params: vec![first],
                                ret: Box::new(ret),
                            },
                            span,
                        ));
                    }
                    let span = start.cover(first.span);
                    return Ok(Spanned::new(TypeExpr::Grouping(Box::new(first)), span));
                }

                // Comma present → could be Func type if ThinArrow follows
                let mut params = vec![first];
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    if matches!(self.peek(), Token::RParen) {
                        break;
                    }
                    params.push(self.parse_type_expr()?);
                }
                self.expect(&Token::RParen, "`)`")?;

                // Check for `->` (function type)
                if matches!(self.peek(), Token::ThinArrow) {
                    self.advance();
                    let ret = self.parse_type_expr()?;
                    let span = start.cover(ret.span);
                    return Ok(Spanned::new(
                        TypeExpr::Func {
                            params,
                            ret: Box::new(ret),
                        },
                        span,
                    ));
                }

                // No `->` — this could be ParamApp but we don't have the type name here.
                // For Fio 1, this case shouldn't arise. Return as grouping of first param
                // as a fallback (this is a simplification).
                let span = start.cover(params.last().expect("non-empty params").span);
                Ok(Spanned::new(
                    TypeExpr::Grouping(Box::new(
                        params.into_iter().next().expect("at least one param"),
                    )),
                    span,
                ))
            }
            _ => Err(self.error("type expression")),
        }
    }
}
