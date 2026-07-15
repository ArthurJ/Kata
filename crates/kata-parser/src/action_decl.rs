//! Parse `action nome (params) -> ret` com body indentado.

use kata_ast::{ActionStmt, Directive, Item, Spanned, Token, TypeExpr};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    /// Parse `action nome (params) -> ret` com body indentado.
    ///
    /// Sintaxe:
    /// ```text
    /// action greet
    ///     echo!("hello")
    ///     echo!("world")
    /// ```
    ///
    /// Sem parênteses de params = Action sem parâmetros.
    /// Sem `-> ret` = retorno Unit (padrão).
    pub fn parse_action_decl(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        self.expect(&Token::Action, "`action`")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("action name")),
        };

        // Params opcionais: `action nome (T1 T2) -> Ret`
        // ou `action nome -> Ret`
        let mut params = Vec::new();
        let ret;

        if matches!(self.peek(), Token::LParen) {
            self.advance(); // consume (
            // Parse type params until )
            while !matches!(self.peek(), Token::RParen | Token::Eof) {
                params.push(self.parse_type_expr()?);
            }
            self.expect(&Token::RParen, "`)` (action params)")?;

            // `->` Ret
            if matches!(self.peek(), Token::ThinArrow) {
                self.advance();
                ret = self.parse_type_expr()?;
            } else {
                // Sem `->` = Unit
                ret = Spanned::new(TypeExpr::Unit, self.peek_span());
            }
        } else if matches!(self.peek(), Token::ThinArrow) {
            self.advance();
            ret = self.parse_type_expr()?;
        } else {
            // Sem params, sem `->` = Action sem parâmetros, retorna Unit
            ret = Spanned::new(TypeExpr::Unit, self.peek_span());
        }

        // Body indentado
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

        Ok(Item::ActionDecl {
            name,
            params,
            ret,
            directives,
            body,
        })
    }
}
