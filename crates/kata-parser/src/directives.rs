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
            // Literais compostos (Fio 14): tupla e variant.
            //
            // Reusa a lógica de parse_directive_args para tuplas (recursivo),
            // mas produz DirectiveValue::Tuple em vez de Vec<DirectiveArg>.
            // O `()` vazio produz tupla vazia (diferente do @test() que produz
            // Vec<DirectiveArg> vazio — mas isso é no nível do arg, não do value).
            Token::LParen => {
                self.advance(); // consume (
                let mut elems = Vec::new();
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    return Ok(DirectiveValue::Tuple(elems));
                }
                loop {
                    elems.push(self.parse_directive_value()?);
                    match self.peek() {
                        Token::Comma => {
                            self.advance();
                        }
                        Token::RParen => {
                            self.advance();
                            break;
                        }
                        _ => return Err(self.error("`,` or `)` in tuple")),
                    }
                }
                Ok(DirectiveValue::Tuple(elems))
            }
            // Variant: `Enum::Variante` ou `Enum::Variante(args)`.
            // O lexer não distingue Ident uppercase de lowercase — detectamos
            // variant pelo padrão Ident :: Ident (mesma heurística de
            // parse_atom em expressions.rs:151-166).
            Token::Ident(enum_name) if matches!(self.tokens.get(self.pos + 1), Some(t) if t.token == Token::DoubleColon) => {
                let enum_name = enum_name.clone();
                self.advance(); // consume enum name
                self.advance(); // consume ::
                let variant = match self.peek() {
                    Token::Ident(v) => {
                        let v = v.clone();
                        self.advance();
                        v
                    }
                    _ => return Err(self.error("variant name after `::`")),
                };
                // Opcional: args da variante entre parênteses.
                let args = if matches!(self.peek(), Token::LParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if matches!(self.peek(), Token::RParen) {
                        self.advance();
                        return Ok(DirectiveValue::Variant(enum_name + "::" + &variant, args));
                    }
                    loop {
                        args.push(self.parse_directive_value()?);
                        match self.peek() {
                            Token::Comma => {
                                self.advance();
                            }
                            Token::RParen => {
                                self.advance();
                                break;
                            }
                            _ => return Err(self.error("`,` or `)` in variant args")),
                        }
                    }
                    args
                } else {
                    Vec::new()
                };
                Ok(DirectiveValue::Variant(enum_name + "::" + &variant, args))
            }
            _ => Err(self.error("string, integer, tuple, or variant value")),
        }
    }
}
