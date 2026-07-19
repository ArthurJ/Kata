//! Directives — parse_directives, parse_directive_args, parse_directive_value.

use kata_ast::{Directive, DirectiveArg, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    // ── Directives ────────────────────────────────────────────────

    pub(crate) fn parse_directives(&mut self) -> Result<Vec<Directive>, FrontendError> {
        let mut directives = Vec::new();
        loop {
            // Skip statement separators between stacked directives
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if !matches!(self.peek(), Token::At) {
                break;
            }
            let at_span = self.advance(); // consume @
            // Expect an identifier (directive name).
            let name = match self.peek() {
                Token::Ident(s) => {
                    let name = s.clone();
                    self.advance();
                    name
                }
                _ => {
                    return Err(self.error("directive name after @"));
                }
            };
            let args = self.parse_directive_args()?;
            directives.push(Directive {
                name,
                args,
                span: at_span,
            });
        }
        Ok(directives)
    }

    fn parse_directive_args(&mut self) -> Result<Vec<DirectiveArg>, FrontendError> {
        let mut args = Vec::new();
        // @name("arg") — parenthesized positional args
        // @name{key: value} — braced named args
        match self.peek() {
            Token::LParen => {
                self.advance(); // consume (
                // Empty parens: no args
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    return Ok(args);
                }
                loop {
                    let arg = self.parse_one_directive_arg()?;
                    args.push(arg);
                    match self.peek() {
                        Token::Comma => {
                            self.advance();
                        }
                        Token::RParen => {
                            self.advance();
                            break;
                        }
                        _ => return Err(self.error("`,` or `)`")),
                    }
                }
            }
            Token::LBrace => {
                self.advance(); // consume {
                if matches!(self.peek(), Token::RBrace) {
                    self.advance();
                    return Ok(args);
                }
                loop {
                    // key : value
                    // `timeout` é keyword do lexer (Token::Timeout) — aceitar
                    // ambas como key (Token::Ident para demais chaves).
                    let key = match self.peek() {
                        Token::Ident(s) => {
                            let k = s.clone();
                            self.advance();
                            k
                        }
                        Token::Timeout => {
                            self.advance();
                            "timeout".to_string()
                        }
                        _ => return Err(self.error("directive key (identifier)")),
                    };
                    self.expect(&Token::Colon, "`:`")?;
                    let value = self.parse_directive_value()?;
                    args.push(DirectiveArg::Named { key, value });
                    match self.peek() {
                        Token::Comma => {
                            self.advance();
                        }
                        Token::RBrace => {
                            self.advance();
                            break;
                        }
                        _ => return Err(self.error("`,` or `}`")),
                    }
                }
            }
            _ => {}
        }
        Ok(args)
    }

    fn parse_one_directive_arg(&mut self) -> Result<DirectiveArg, FrontendError> {
        // D2: posicional é Expr livre (mesmo conjunto que nomeado).
        // Reusa parse_apply — aceita literais, tupla, variant, apply posicional
        // de construtor, mas não operadores de pipe/|>/| que não fazem sentido
        // em valor de diretiva. O consumer (@ffi, @test) valida o tipo de Expr.
        let expr = crate::expr_apply::parse_apply(self)?;
        Ok(DirectiveArg::Expr(Box::new(expr)))
    }

    fn parse_directive_value(&mut self) -> Result<Box<Spanned<kata_ast::Expr>>, FrontendError> {
        // D2: valor de dict é Expr livre (mesmo conjunto que posicional).
        // Reusa parse_apply — aceita tupla, variant, apply posicional de
        // construtor. O consumer (@test, @cache_strategy) valida o tipo.
        let expr = crate::expr_apply::parse_apply(self)?;
        Ok(Box::new(expr))
    }
}
