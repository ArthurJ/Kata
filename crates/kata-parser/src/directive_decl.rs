//! Parse `directive name{when: Hook::Enter, on: Target::Action}` com body.
//!
//! Sintaxe:
//! ```text
//! directive log{when: Hook::Enter, on: Target::Action}
//!     let _ := print!(_name)
//! ```
//!
//! O header reusa `parse_directive_args` (ramo `{}`) para o dict.
//! O body é parseado como `Vec<ActionStmt>` (mesmo que `ActionDecl`).

use kata_ast::{ActionStmt, Item, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    /// Parse `directive name{when: ..., on: ...}` com body indentado.
    pub(crate) fn parse_directive_decl(&mut self) -> Result<Item, FrontendError> {
        self.expect(&Token::Directive, "`directive`")?;

        // Nome da diretiva
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("directive name")),
        };

        // Args: {when: ..., on: ...} — reusa parse_directive_args
        let args = self.parse_directive_args()?;

        // Body indentado — mesmo padrão de ActionDecl
        self.expect(&Token::Indent, "INDENT (directive body)")?;
        let prev_in_action = self.in_action_body;
        self.in_action_body = true;
        let mut body = Vec::new();
        loop {
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            let stmt = parse_expr(self)?;
            let has_semicolon = matches!(self.peek(), Token::Semicolon);
            if has_semicolon {
                self.advance();
            }
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            body.push(ActionStmt {
                expr: stmt,
                has_semicolon,
            });
        }
        self.in_action_body = prev_in_action;
        self.expect(&Token::Dedent, "DEDENT (fim do directive body)")?;

        Ok(Item::DirectiveDecl { name, args, body })
    }
}
