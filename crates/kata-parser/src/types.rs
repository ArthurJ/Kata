//! Type expressions — Named, Unit, Grouping, Func.

use kata_ast::{Spanned, Token, TypeExpr};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    pub(crate) fn parse_type_expr(&mut self) -> Result<Spanned<TypeExpr>, FrontendError> {
        let ty = self.parse_type_expr_inner()?;
        // `T?` — açúcar sintático para `Result::(T, Err)`.
        // Postfix: liga-se ao tipo imediatamente anterior. Pode encadear:
        // `T??` = `Result::(Result::(T, Err), Err)`.
        let mut ty = ty;
        while matches!(self.peek(), Token::Question) {
            let q_span = self.advance();
            let span = ty.span.cover(q_span);
            ty = Spanned::new(TypeExpr::Question(Box::new(ty)), span);
        }
        Ok(ty)
    }

    fn parse_type_expr_inner(&mut self) -> Result<Spanned<TypeExpr>, FrontendError> {
        let start = self.peek_span();

        // Action(Params) => Ret — tipo de Action first-class.
        if matches!(self.peek(), Token::Ident(name) if name == "Action") {
            self.advance(); // consome "Action"
            self.expect(&Token::LParen, "\"(\" após Action")?;
            let mut params = Vec::new();
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::RParen) {
                self.advance();
            } else {
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
            }
            self.expect(&Token::FatArrow, "'=>' após Action(params)")?;
            let ret = self.parse_type_expr()?;
            let span = start.cover(ret.span);
            return Ok(Spanned::new(
                TypeExpr::ActionType {
                    params,
                    ret: Box::new(ret),
                },
                span,
            ));
        }

        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance();
                // `Self` — referência ao tipo que implementa a interface.
                if name == "Self" {
                    return Ok(Spanned::new(TypeExpr::SelfRef, start));
                }
                // `Name::(T1, T2)` — tipo com parâmetros posicionais (tupla).
                // `Name::T` — tipo com um parâmetro (single-param, sem parênteses).
                if matches!(self.peek(), Token::DoubleColon) {
                    self.advance(); // consume ::
                    if matches!(self.peek(), Token::LParen) {
                        // `Name::(T1, T2)` — parênteses delimitam tupla de tipos.
                        self.advance(); // consume (
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
                    } else {
                        // `Name::T` — single-param sem parênteses.
                        let ty = self.parse_type_expr()?;
                        let span = start.cover(ty.span);
                        return Ok(Spanned::new(
                            TypeExpr::ParamApp {
                                name,
                                params: vec![ty],
                            },
                            span,
                        ));
                    }
                }
                // `module.Type` — tipo qualificado de módulo importado.
                // Após consumir o Ident, se há `.` segue outro Ident.
                if matches!(self.peek(), Token::Dot) {
                    self.advance(); // consome .
                    if let Token::Ident(type_name) = self.peek().clone() {
                        let dot_end = self.advance();
                        let span = start.cover(dot_end);
                        return Ok(Spanned::new(
                            TypeExpr::Qualified {
                                module: name,
                                name: type_name,
                            },
                            span,
                        ));
                    } else {
                        return Err(self.error("type name after `.`"));
                    }
                }
                Ok(Spanned::new(TypeExpr::Named(name), start))
            }
            Token::LBracket => {
                // `[T]` — açúcar sintático para `List::T`.
                // Desugara no mesmo TypeExpr::ParamApp que `List::T` produz.
                self.advance(); // consume [
                let ty = self.parse_type_expr()?;
                self.expect(&Token::RBracket, "\"]\"")?;
                let span = start.cover(ty.span);
                Ok(Spanned::new(
                    TypeExpr::ParamApp {
                        name: "List".into(),
                        params: vec![ty],
                    },
                    span,
                ))
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
