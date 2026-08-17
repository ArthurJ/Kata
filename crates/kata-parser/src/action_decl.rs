//! Parse `action nome (p1::T1, p2::T2, ...) => Ret` com body indentado.
//!
//! Sintaxe (após migração total para params nomeados):
//! ```text
//! action soma (x::Int, y::Int) => Int
//!     + x y
//! ```
//!
//! Formas sem params:
//! - `action greet` — sem params, retorna Unit.
//! - `action greet => Unit` — sem params, retorno explícito.
//!
//! A forma posicional legada (`action nome (T1 T2) -> Ret`) foi removida.

use kata_ast::{ActionStmt, Directive, Expr, Item, Spanned, Token, TypeExpr};
use kata_diagnostics::FrontendError;

use crate::CasingPattern;
use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    /// Parse `action nome (p::T, ...) => Ret` com body indentado.
    pub(crate) fn parse_action_decl(
        &mut self,
        directives: Vec<Directive>,
    ) -> Result<Item, FrontendError> {
        self.expect(&Token::Action, "`action`")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let span = self.peek_span();
                let n = s.clone();
                self.advance();
                self.validate_name(&n, CasingPattern::SnakeCase, span)?;
                n
            }
            _ => return Err(self.error("action name")),
        };

        let mut params = Vec::new();
        let mut param_names = Vec::new();
        let mut param_defaults: Vec<Option<Spanned<Expr>>> = Vec::new();
        let ret;

        if matches!(self.peek(), Token::LParen) {
            self.advance(); // consume (
            // Skip newlines após `(`
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }

            // Forma nomeada: (x::Tipo, y::Tipo, ...) => Ret
            // Açúcar para {x::Tipo: _, y::Tipo: _} — todos obrigatórios.
            loop {
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                if matches!(self.peek(), Token::RParen) {
                    break;
                }

                // Espera: Ident :: Tipo
                let pname = match self.peek() {
                    Token::Ident(s) => {
                        let span = self.peek_span();
                        let n = s.clone();
                        self.advance();
                        self.validate_name(&n, CasingPattern::SnakeCase, span)?;
                        n
                    }
                    _ => return Err(self.error("nome do parâmetro")),
                };
                self.expect(&Token::DoubleColon, "`::` após nome do parâmetro")?;
                let ty = self.parse_type_expr()?;
                params.push(ty);
                param_names.push(Some(pname));
                param_defaults.push(None); // (x::Int) = obrigatório

                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    continue;
                }
                break;
            }
            self.expect(&Token::RParen, "`)` (action params)")?;

            // `=>` Ret
            if matches!(self.peek(), Token::FatArrow) {
                self.advance();
                ret = self.parse_type_expr()?;
            } else {
                ret = Spanned::new(TypeExpr::Unit, self.peek_span());
            }
        } else if matches!(self.peek(), Token::LBrace) {
            // Dict-template: {x::Tipo: _, y::Tipo: 5, ...} => Ret
            // `_` = obrigatório, literal/expr = default.
            self.advance(); // consume {
            // Skip newlines após `{`
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }

            loop {
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                if matches!(self.peek(), Token::RBrace) {
                    break;
                }

                // Espera: Ident :: Tipo : valor
                let pname = match self.peek() {
                    Token::Ident(s) => {
                        let span = self.peek_span();
                        let n = s.clone();
                        self.advance();
                        self.validate_name(&n, CasingPattern::SnakeCase, span)?;
                        n
                    }
                    _ => return Err(self.error("nome do parâmetro no dict-template")),
                };
                self.expect(&Token::DoubleColon, "`::` após nome do parâmetro")?;
                let ty = self.parse_type_expr()?;
                self.expect(
                    &Token::Colon,
                    "`:` separando tipo e default/obrigatório no dict-template",
                )?;

                // Valor: `_` (Hole = obrigatório) ou expressão (default)
                let default_val = parse_expr(self)?;
                let default = match &default_val.node {
                    Expr::Hole => None,
                    _ => Some(default_val),
                };

                params.push(ty);
                param_names.push(Some(pname));
                param_defaults.push(default);

                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    continue;
                }
                break;
            }
            self.expect(&Token::RBrace, "`}` (fim do dict-template)")?;

            // `=>` Ret
            if matches!(self.peek(), Token::FatArrow) {
                self.advance();
                ret = self.parse_type_expr()?;
            } else {
                ret = Spanned::new(TypeExpr::Unit, self.peek_span());
            }
        } else if matches!(self.peek(), Token::FatArrow) {
            self.advance();
            ret = self.parse_type_expr()?;
        } else {
            // Sem params, sem `=>` = Action sem parâmetros, retorna Unit
            ret = Spanned::new(TypeExpr::Unit, self.peek_span());
        }

        // Body indentado — opcional se a Action tem @ffi (builtin FFI).
        let has_ffi = directives.iter().any(|d| d.name == "ffi");
        let body = if has_ffi && !matches!(self.peek(), Token::Indent) {
            // Action FFI builtin sem body — ex: @ffi("kata_rt_print")
            //                          action echo (msg::Text) => Unit
            Vec::new()
        } else {
            self.expect(&Token::Indent, "INDENT (action body)")?;
            let prev_in_action = self.in_action_body;
            self.in_action_body = true;
            let mut body = Vec::new();
            loop {
                // Skip leading statement separators (newlines) — but NOT semicolons,
                // because a semicolon after an expression is a meaningful mark.
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                if matches!(self.peek(), Token::Dedent | Token::Eof) {
                    break;
                }
                let stmt = parse_expr(self)?;
                // Check for trailing `;` — marks this statement as local computation.
                let has_semicolon = matches!(self.peek(), Token::Semicolon);
                if has_semicolon {
                    self.advance();
                }
                // Consume any trailing StmtSep after the statement (or after `;`).
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                body.push(ActionStmt {
                    expr: stmt,
                    has_semicolon,
                });
            }
            self.in_action_body = prev_in_action;
            self.expect(&Token::Dedent, "DEDENT (fim do action body)")?;
            body
        };

        Ok(Item::ActionDecl {
            name,
            params,
            param_names,
            param_defaults,
            ret,
            directives,
            body,
        })
    }
}
