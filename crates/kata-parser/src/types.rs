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

                // Coleta tipos até `->` ou `)`. Sintaxe: `(A B C -> D)`.
                // Params separados por espaço, `->` separa params do retorno.
                let mut params = Vec::new();

                loop {
                    let ty = self.parse_type_expr()?;

                    if matches!(self.peek(), Token::ThinArrow) {
                        // `ty ->` — ty é o último param, próximo é o retorno.
                        params.push(ty);
                        self.advance(); // consume ->
                        let ret = self.parse_type_expr()?;
                        self.expect(&Token::RParen, "\")\"")?;
                        let span = start.cover(ret.span);
                        return Ok(Spanned::new(
                            TypeExpr::Func {
                                params,
                                ret: Box::new(ret),
                            },
                            span,
                        ));
                    }

                    if matches!(self.peek(), Token::RParen) {
                        // `ty)` — sem `->`, é grouping de um único tipo.
                        if params.is_empty() {
                            self.advance(); // consume )
                            let span = start.cover(ty.span);
                            return Ok(Spanned::new(TypeExpr::Grouping(Box::new(ty)), span));
                        }
                        // Múltiplos tipos sem `->` — não é válido em código
                        // bem-formado. Retornamos grouping do primeiro como
                        // fallback (não deveria acontecer).
                        params.push(ty);
                        self.advance(); // consume )
                        let span = start.cover(params.last().expect("non-empty").span);
                        return Ok(Spanned::new(
                            TypeExpr::Grouping(Box::new(
                                params.into_iter().next().expect("at least one"),
                            )),
                            span,
                        ));
                    }

                    params.push(ty);
                }
            }
            _ => Err(self.error("type expression")),
        }
    }
}