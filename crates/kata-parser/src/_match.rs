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
            Some(self.parse_match_pattern()?)
        };

        // Expect `:`
        self.expect(&Token::Colon, "`:` após pattern do braço")?;

        // Se o body está indentado (INDENT antes da expressão), consumir
        // o INDENT e parsear um bloco de statements separados por StmtSep.
        // Isto permite braços com match/let/select aninhados em linha
        // separada, com múltiplas statements antes da expressão final:
        //   match x
        //     Result::Ok h:
        //       let f2 := open!(...)
        //       match f2
        //         Result::Ok h2: ...
        // O INDENT é emitido pelo lexer quando o body está mais indentado
        // que o pattern. Sem isto, parse_expr vê INDENT e falha.
        let has_indent = matches!(self.peek(), Token::Indent);
        let body = if has_indent {
            self.advance(); // consome INDENT

            // Parsear statements separados por StmtSep.
            // Se há apenas uma statement, retorná-la diretamente.
            // Se há múltiplas, produzir Expr::Block.
            let mut stmts: Vec<Spanned<Expr>> = Vec::new();

            loop {
                // Consome StmtSep iniciais
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                if matches!(self.peek(), Token::Dedent | Token::Eof) {
                    break;
                }
                let stmt = parse_expr(self)?;
                stmts.push(stmt);
                // Consome StmtSep após statement
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
            }

            // Consome o DEDENT do bloco indentado.
            if matches!(self.peek(), Token::Dedent) {
                self.advance();
            }

            // Se há apenas uma statement, retorná-la diretamente (sem Block).
            // Se há múltiplas, envolver em Block.
            if stmts.len() == 1 {
                stmts.into_iter().next().unwrap()
            } else {
                Spanned::new(Expr::Block { stmts }, arm_start)
            }
        } else {
            // Body começa na mesma linha do pattern.
            let first_stmt = parse_expr(self)?;

            // Caso misto: o body começou na mesma linha mas continua em
            // linhas indentadas. Ex:
            //   Result::Ok h: let x := 10
            //     + x h
            // O lexer emite INDENT antes de `+ x h` porque a indentação
            // aumentou. Sem este tratamento, o INDENT seria interpretado
            // como início do próximo braço pelo loop externo do parse_match.
            if matches!(self.peek(), Token::Indent) {
                self.advance(); // consome INDENT

                let mut stmts = vec![first_stmt];
                loop {
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    if matches!(self.peek(), Token::Dedent | Token::Eof) {
                        break;
                    }
                    let stmt = parse_expr(self)?;
                    stmts.push(stmt);
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                }

                if matches!(self.peek(), Token::Dedent) {
                    self.advance();
                }

                if stmts.len() == 1 {
                    stmts.into_iter().next().unwrap()
                } else {
                    Spanned::new(Expr::Block { stmts }, arm_start)
                }
            } else {
                // Body de expressão única na mesma linha.
                if matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                first_stmt
            }
        };

        let _ = arm_start; // span do braço não é armazenado em MatchArm
        Ok(MatchArm {
            pattern,
            guard: None, // Guard opcional após pattern (não implementado)
            body,
        })
    }
}
