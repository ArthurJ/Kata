//! Match — parse_match com braços indentados.
//!
//! Sintaxe:
//! ```text
//! match <scrutinee>
//!     <pattern>: <body>
//!     <pattern>: <body>
//!     otherwise: <body>
//! ```

use kata_ast::{Expr, MatchArm, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    /// Parse `match <scrutinee>` com braços indentados.
    ///
    /// Cada braço: `<pattern>: <body>` ou `otherwise: <body>`.
    pub(crate) fn parse_match(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Match, "`match`")?;

        // Parse scrutinee (expressão após `match`)
        let scrutinee = parse_expr(self)?;

        // Expect INDENT
        self.expect(&Token::Indent, "INDENT (braços do match)")?;

        // Parse arms
        let mut arms = Vec::new();
        loop {
            // Skip StmtSep entre braços
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }

            let arm = self.parse_match_arm()?;
            arms.push(arm);
        }

        self.expect(&Token::Dedent, "DEDENT (fim do match)")?;

        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(
            Expr::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span,
        ))
    }

    /// Parse um braço de match: `<pattern>: <body>` ou `otherwise: <body>`.
    fn parse_match_arm(&mut self) -> Result<MatchArm, FrontendError> {
        let arm_start = self.peek_span();

        // `otherwise` → fallback (pattern = None)
        let pattern = if matches!(self.peek(), Token::Otherwise) {
            self.advance(); // consume `otherwise`
            None
        } else {
            Some(self.parse_pattern()?)
        };

        // Expect `:`
        self.expect(&Token::Colon, "`:` após pattern do braço")?;

        // Parse body (expressão)
        let body = parse_expr(self)?;

        // Consome StmtSep se presente
        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        let _ = arm_start; // span do braço não é armazenado em MatchArm
        Ok(MatchArm {
            pattern,
            guard: None, // Fio 2: guard opcional após pattern (não implementado)
            body,
        })
    }
}
