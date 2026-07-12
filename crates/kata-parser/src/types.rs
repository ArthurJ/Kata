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
                // `Name::(T1, T2)` — tipo com parâmetros posicionais.
                if matches!(self.peek(), Token::DoubleColon) {
                    self.advance(); // consume ::
                    self.expect(&Token::LParen, "\"(\"")?;
                    let mut params = Vec::new();
                    // Skip newlines after (
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    if matches!(self.peek(), Token::RParen) {
                        let rparen_span = self.advance();
                        let span = start.cover(rparen_span);
                        return Ok(Spanned::new(TypeExpr::ParamApp { name, params }, span));
                    }
                    loop {
                        let ty = self.parse_type_expr()?;
                        params.push(ty);
                        if matches!(self.peek(), Token::Comma) {
                            self.advance();
                            while matches!(self.peek(), Token::StmtSep) {
                                self.advance();
                            }
                            continue;
                        }
                        break;
                    }
                    self.expect(&Token::RParen, "\")\"")?;
                    let last_span = params.last().expect("non-empty").span;
                    let span = start.cover(last_span);
                    return Ok(Spanned::new(TypeExpr::ParamApp { name, params }, span));
                }
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
                // Com vírgulas: `(T1, T2, ...)` → TypeExpr::Tuple.
                let mut params = Vec::new();
                let mut has_comma = false;

                loop {
                    let ty = self.parse_type_expr()?;
                    params.push(ty);

                    if matches!(self.peek(), Token::Comma) {
                        has_comma = true;
                        self.advance(); // consume ,
                        // Skip newlines after comma (multiline tuple types)
                        while matches!(self.peek(), Token::StmtSep) {
                            self.advance();
                        }
                        continue;
                    }

                    if matches!(self.peek(), Token::ThinArrow) {
                        // `ty ->` — ty é o último param, próximo é o retorno.
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
                        self.advance(); // consume )
                        let last_span = params.last().expect("non-empty").span;
                        let span = start.cover(last_span);
                        if has_comma {
                            // `(T1, T2, ...)` — tipo tupla
                            return Ok(Spanned::new(TypeExpr::Tuple(params), span));
                        }
                        if params.len() == 1 {
                            // `(ty)` — grouping de um único tipo.
                            return Ok(Spanned::new(
                                TypeExpr::Grouping(Box::new(
                                    params.into_iter().next().expect("at least one"),
                                )),
                                span,
                            ));
                        }
                        // Múltiplos tipos sem `->` e sem vírgula — não é válido
                        // em código bem-formado. Retornamos grouping do primeiro.
                        return Ok(Spanned::new(
                            TypeExpr::Grouping(Box::new(
                                params.into_iter().next().expect("at least one"),
                            )),
                            span,
                        ));
                    }

                    // Continua coletando (espaço entre params)
                }
            }
            _ => Err(self.error("type expression")),
        }
    }
}
