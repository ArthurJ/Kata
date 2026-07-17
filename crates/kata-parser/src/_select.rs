//! Select — parse_select com braços de canal e timeout (Fio 11).
//!
//! Sintaxe:
//! ```text
//! select
//!     rx <! msg: echo!(msg)
//!     rx2 <! item: handle!(item)
//!     timeout 5000: echo!("timeout")
//! ```
//!
//! Cada braço é `receiver <! binding_name: body`.
//! O braço `timeout N: body` é opcional e sempre o último.
//!
//! Como `<!` é um operador infixo em `parse_expr`, o parser de select
//! chama `parse_expr` que produz `Expr::ChannelRecv { channel, bind_name }`
//! para cada braço. O `: body` é então parseado separadamente.

use kata_ast::{Expr, SelectArm, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    /// Parse `select` com braços indentados.
    ///
    /// Cada braço: `receiver <! nome: body` ou `timeout N: body`.
    pub(crate) fn parse_select(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Select, "`select`")?;

        // Expect INDENT
        self.expect(&Token::Indent, "INDENT (braços do select)")?;

        let mut arms = Vec::new();
        let mut timeout_ms = None;
        let mut timeout_body = None;

        loop {
            // Skip StmtSep entre braços
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }

            if matches!(self.peek(), Token::Timeout) {
                // `timeout N: body`
                self.advance(); // consume `timeout`
                let ms = parse_expr(self)?;
                self.expect(&Token::Colon, "`:` após timeout N")?;
                let body = parse_expr(self)?;
                timeout_ms = Some(Box::new(ms));
                timeout_body = Some(Box::new(body));
            } else {
                // `receiver <! nome: body`
                // parse_expr consome `rx <! nome` como Expr::ChannelRecv.
                let recv_expr = parse_expr(self)?;

                // Extrai channel e bind_name do ChannelRecv
                let (channel, bind_name) = match recv_expr.node {
                    Expr::ChannelRecv { channel, bind_name } => (*channel, bind_name),
                    _ => {
                        return Err(self.error(
                            "esperado `receiver <! nome` no braço do select",
                        ));
                    }
                };

                self.expect(&Token::Colon, "`:` após binding do select")?;
                let body = parse_expr(self)?;

                arms.push(SelectArm {
                    channel,
                    bind_name,
                    body,
                });
            }

            // Consome StmtSep se presente
            if matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
        }

        self.expect(&Token::Dedent, "DEDENT (fim do select)")?;

        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(
            Expr::Select {
                arms,
                timeout_ms,
                timeout_body,
            },
            span,
        ))
    }
}