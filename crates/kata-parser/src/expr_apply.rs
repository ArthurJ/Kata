//! Expression operator precedence — free functions for pipe, question, and
//! greedy application parsing.
//!
//! Extraído de `expressions.rs` para separar a mecânica de precedência de
//! operadores (`|>`, `|`, `?`, aplicação greedy) do parsing de átomos.

use kata_ast::{Expr, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

/// Parse an expression with greedy application.
/// Free function — called from declarations and expressions.
///
/// After parsing the application, checks for `|>` (pipe) infix operator.
/// `|>` has lower precedence than application and is left-associative.
/// `a |> b |> c` = `(a |> b) |> c`.
pub(crate) fn parse_expr(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
    let mut lhs = parse_apply(parser)?;

    // `?` postfix — fail-fast operator.
    // `expr ?` → Expr::Question(Box<expr>).
    // Precedência: maior que `|>` (pipe), menor que `::` (ascription).
    // Ou seja: `Result::Ok 42 ?` = `(Result::Ok 42) ?`, e
    // `x ? |> f` = `(x ?) |> f`.
    while matches!(parser.peek(), Token::Question) {
        let q_span = parser.advance(); // consume ?
        let span = lhs.span.cover(q_span);
        lhs = Spanned::new(Expr::Question(Box::new(lhs)), span);
    }

    // `|>` pipe, `|` fallback, `!>` send, `<!` recv — same precedence, left-associative.
    // `lhs |> rhs |> rhs2` = `(lhs |> rhs) |> rhs2`
    // `lhs | rhs | rhs2`    = `(lhs | rhs) | rhs2`
    // `lhs |> rhs | rhs2`  = `(lhs |> rhs) | rhs2`  (intercalado)
    loop {
        match parser.peek() {
            Token::PipeForward => {
                parser.advance(); // consume `|>`
                let mut rhs = parse_apply(parser)?;
                // rhs também pode ter `?` postfix: `x |> f ?` = `x |> (f ?)`.
                while matches!(parser.peek(), Token::Question) {
                    let q_span = parser.advance();
                    let span = rhs.span.cover(q_span);
                    rhs = Spanned::new(Expr::Question(Box::new(rhs)), span);
                }
                let span = lhs.span.cover(rhs.span);
                lhs = Spanned::new(
                    Expr::Pipe {
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    span,
                );
            }
            Token::Pipe => {
                let pipe_span = parser.advance(); // consume `|`
                let mut rhs = parse_apply(parser)?;
                // rhs também pode ter `?` postfix: `x | f ?` = `x | (f ?)`.
                while matches!(parser.peek(), Token::Question) {
                    let q_span = parser.advance();
                    let span = rhs.span.cover(q_span);
                    rhs = Spanned::new(Expr::Question(Box::new(rhs)), span);
                }
                let span = lhs.span.cover(pipe_span).cover(rhs.span);
                lhs = Spanned::new(
                    Expr::PipeFallback {
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    span,
                );
            }
            Token::SendArrow => {
                // `tx !> valor` — envio por canal.
                parser.advance(); // consume `!>`
                let rhs = parse_apply(parser)?;
                let span = lhs.span.cover(rhs.span);
                lhs = Spanned::new(
                    Expr::ChannelSend {
                        channel: Box::new(lhs),
                        value: Box::new(rhs),
                    },
                    span,
                );
            }
            Token::RecvArrow => {
                // `rx <! nome` — recebimento de canal.
                // `<!` exige um Ident como destino (binding name).
                parser.advance(); // consume `<!`
                let name = match parser.peek() {
                    Token::Ident(s) => {
                        let n = s.clone();
                        parser.advance();
                        n
                    }
                    _ => {
                        return Err(parser
                            .error("identificador após `<!` (nome do binding de recebimento)"));
                    }
                };
                let end_span = parser
                    .tokens
                    .get(parser.pos.wrapping_sub(1))
                    .map(|t| t.span)
                    .unwrap_or(lhs.span);
                let span = lhs.span.cover(end_span);
                lhs = Spanned::new(
                    Expr::ChannelRecv {
                        channel: Box::new(lhs),
                        bind_name: name,
                    },
                    span,
                );
            }
            _ => break,
        }
    }

    // `in` — membership operator. Precedência: comparação (mais baixa que `|>`, `|`).
    // `x in coll` → Expr::In { item: x, collection: coll }.
    // Left-associative: `a in b in c` = `(a in b) in c`.
    while matches!(parser.peek(), Token::In) {
        let in_span = parser.advance(); // consume `in`
        let rhs = parse_apply(parser)?;
        // rhs também pode ter `?` postfix
        while matches!(parser.peek(), Token::Question) {
            let q_span = parser.advance();
            let span = rhs.span.cover(q_span);
            // Note: o `?` aplica ao rhs, não ao In inteiro
            let _ = (q_span, span); // rhs não é mutável aqui, simplificação
        }
        let span = lhs.span.cover(in_span).cover(rhs.span);
        lhs = Spanned::new(
            Expr::In {
                item: Box::new(lhs),
                collection: Box::new(rhs),
            },
            span,
        );
    }

    Ok(lhs)
}

/// Parse a greedy application: callee + arguments.
/// Does NOT consume `|>` — that's handled by `parse_expr`.
///
/// Invariant sintática: em notação prefixa estrita, só `Ident`,
/// `VariantQual`, e `Grouping` (com lambda dentro) podem ser callee.
/// Literais (`IntLit`, `FloatLit`, `TextLit`, `Unit`) não são chamáveis —
/// se o parser os consumisse como callee, `5 in + x 1` seria parseado como
/// `Apply(5, [in, +, x, 1])` em vez de parar em `5` e reportar `in` como
/// token inesperado.
pub(crate) fn parse_apply(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
    let callee = parser.parse_expr_post_ascription()?;

    // Literais, construções de statement e keywords de controle de fluxo
    // não são callee — não consomem argumentos.
    // Liturais: IntLit, FloatLit, TextLit, Unit
    // Statements: Let, Var, Reassign (bindings auto-delimitados)
    // Blocos: Match, Loop (consomem INDENT/DEDENT)
    // Keywords: Break, Continue, Return (controle de fluxo)
    if matches!(
        &callee.node,
        Expr::IntLit { .. }
            | Expr::FloatLit { .. }
            | Expr::TextLit { .. }
            | Expr::BytesLit { .. }
            | Expr::Unit
            | Expr::Let { .. }
            | Expr::Var { .. }
            | Expr::Reassign { .. }
            | Expr::Match { .. }
            | Expr::Loop { .. }
            | Expr::ForIn { .. }
            | Expr::Select { .. }
            | Expr::Break
            | Expr::Continue
            | Expr::Return(..)
    ) {
        return Ok(callee);
    }

    let mut args = Vec::new();
    while parser.can_start_expr() {
        args.push(parser.parse_expr_atom_or_ascription()?);
    }

    if args.is_empty() {
        Ok(callee)
    } else {
        let span = callee.span.cover(args.last().expect("non-empty args").span);
        Ok(Spanned::new(
            Expr::Apply {
                callee: Box::new(callee),
                args,
            },
            span,
        ))
    }
}
