//! Directives — parse_directives, parse_directive_args, parse_directive_value.

use kata_ast::{Directive, DirectiveArg, DirectiveValue, Token};
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
            // Expect an identifier (directive name)
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
                    let key = match self.peek() {
                        Token::Ident(s) => {
                            let k = s.clone();
                            self.advance();
                            k
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
        match self.peek() {
            Token::TextLit(s) => {
                let val = s.clone();
                self.advance();
                Ok(DirectiveArg::Str(val))
            }
            Token::IntLit(s) => {
                let val: i64 = s.parse().map_err(|_| self.error("integer literal"))?;
                self.advance();
                Ok(DirectiveArg::Int(val))
            }
            _ => Err(self.error("string or integer argument")),
        }
    }

    fn parse_directive_value(&mut self) -> Result<DirectiveValue, FrontendError> {
        match self.peek() {
            Token::TextLit(s) => {
                let val = s.clone();
                self.advance();
                Ok(DirectiveValue::Str(val))
            }
            Token::IntLit(s) => {
                let val: i64 = s.parse().map_err(|_| self.error("integer literal"))?;
                self.advance();
                Ok(DirectiveValue::Int(val))
            }
            _ => Err(self.error("string or integer value")),
        }
    }
}
