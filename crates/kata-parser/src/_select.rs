//! Select — parse_select com braços de canal, I/O e timeout.
//!
//! Sintaxe:
//! ```text
//! select
//!     rx !> msg: echo!(msg)
//!     read!(file, 4096) !> data: processa!(data)
//!     timeout 5000: echo!("timeout")
//! ```
//!
//! Cada braço é `receiver !> binding_name: body` ou
//! `read!(handle, n) !> binding_name: body`.
//! O braço `timeout N: body` é opcional e sempre o último.
//!
//! Como `!>` é um operador infixo em `parse_expr`, o parser de select
//! chama `parse_expr` que produz `Expr::ChannelRecv { channel, bind_name }`
//! para cada braço. O `: body` é então parseado separadamente.
//!
//! Para distinguir braços de canal de braços de I/O, o parser inspeciona
//! o `channel` dentro do `ChannelRecv`: se for `ActionCall { callee: "read", ... }`,
//! é um braço `IoRead`; caso contrário, é um braço `Channel`.

use kata_ast::{Expr, ReadMode, SelectArm, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    /// Parse `select` com braços indentados.
    ///
    /// Cada braço: `receiver !> nome: body` ou `read!(handle, n) !> nome: body`
    /// ou `timeout N: body`.
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
                // `receiver !> nome: body` ou `read!(handle, n) !> nome: body`
                // parse_expr consome `expr !> nome` como Expr::ChannelRecv.
                let recv_expr = parse_expr(self)?;

                // Extrai channel e bind_name do ChannelRecv
                let (channel, bind_name) = match recv_expr.node {
                    Expr::ChannelRecv { channel, bind_name } => (*channel, bind_name),
                    _ => {
                        return Err(self.error("esperado `receiver !> nome` no braço do select"));
                    }
                };

                self.expect(&Token::Colon, "`:` após binding do select")?;
                let body = parse_expr(self)?;

                // Distingue braço de canal de braço de I/O.
                // Se o channel é ActionCall { callee: "read", args: (handle, n) },
                // é um braço IoRead com ReadMode::Chunk.
                // Se o channel é ActionCall { callee: "readline", args: handle },
                // é um braço IoRead com ReadMode::Line.
                let arm = match &channel.node {
                    Expr::ActionCall { callee, args } if callee == "read" => {
                        // Extrai handle_expr e chunk_size_expr dos args.
                        // args é uma tupla: (handle, n) ou um valor único.
                        // Para read!(handle, n), args é Tuple([handle, n]).
                        let (handle_expr, chunk_size_expr) = extract_read_args(args);
                        SelectArm::IoRead {
                            handle_expr,
                            read_mode: ReadMode::Chunk(chunk_size_expr),
                            bind_name,
                            body,
                        }
                    }
                    Expr::ActionCall { callee, args } if callee == "readline" => {
                        // readline!(handle) — sem chunk_size.
                        let handle_expr = extract_readline_arg(args);
                        SelectArm::IoRead {
                            handle_expr,
                            read_mode: ReadMode::Line,
                            bind_name,
                            body,
                        }
                    }
                    _ => SelectArm::Channel {
                        channel,
                        bind_name,
                        body,
                    },
                };

                arms.push(arm);
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

/// Extrai `handle_expr` dos args de `readline!(handle)`.
///
/// `args` é a expressão de argumentos do `ActionCall`. Para `readline!(h)`,
/// o parser produz `ActionCall { callee: "readline", args: h }`.
/// Se vier como tupla `(h,)`, extrai o primeiro elemento.
fn extract_readline_arg(args: &Spanned<Expr>) -> Spanned<Expr> {
    match &args.node {
        Expr::Tuple { elements } if elements.len() == 1 => elements[0].clone(),
        _ => args.clone(),
    }
}

/// Extrai `handle_expr` e `chunk_size_expr` dos args de `read!(handle, n)`.
///
/// `args` é a expressão de argumentos do `ActionCall`. Para `read!(h, 4096)`,
/// o parser produz `ActionCall { callee: "read", args: Tuple([h, 4096]) }`.
/// Para `read!(h)` (1 arg), args é apenas `h` (sem tupla).
fn extract_read_args(args: &Spanned<Expr>) -> (Spanned<Expr>, Spanned<Expr>) {
    match &args.node {
        Expr::Tuple { elements } if elements.len() == 2 => {
            // (handle, n) — tuple de 2 elementos.
            let handle = elements[0].clone();
            let chunk_size = elements[1].clone();
            (handle, chunk_size)
        }
        _ => {
            // Fallback: não deveria acontecer em código válido.
            // Usa o args inteiro como handle e 0 como chunk_size.
            (
                args.clone(),
                Spanned::new(
                    Expr::IntLit {
                        text: "0".to_string(),
                    },
                    args.span,
                ),
            )
        }
    }
}
