//! Expression operator precedence — free functions for pipe, question, and
//! greedy application parsing.
//!
//! Extraído de `expressions.rs` para separar a mecânica de precedência de
//! operadores (`|>`, `|`, `?`, aplicação greedy) do parsing de átomos.

use kata_ast::{Expr, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::MAX_EXPR_DEPTH;
use crate::Parser;

/// Parse an expression with greedy application.
/// Free function — called from declarations and expressions.
///
/// After parsing the application, checks for `|>` (pipe) infix operator.
/// `|>` has lower precedence than application and is left-associative.
/// `a |> b |> c` = `(a |> b) |> c`.
pub(crate) fn parse_expr(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
    if parser.depth > MAX_EXPR_DEPTH {
        return Err(FrontendError::NestingTooDeep {
            limit: MAX_EXPR_DEPTH,
            span: kata_diagnostics::MietteSpan(parser.peek_span()),
        });
    }
    parser.depth += 1;
    let result = parse_expr_impl(parser);
    parser.depth -= 1;
    result
}

/// Corpo de `parse_expr` — separado para que o depth guard possa
/// increment/decrementar sem condicionais no caminho feliz.
fn parse_expr_impl(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
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

    // `|>` pipe, `|` fallback, `<!` send, `!>` recv — same precedence, left-associative.
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
            Token::PipeLimit { .. } => {
                let pipe_span = parser.advance(); // consume `|N>`
                // O limit é o texto entre `|` e `>`: literal int ou ident.
                let limit_text =
                    if let Token::PipeLimit { limit } = &parser.tokens[parser.pos - 1].token {
                        limit.clone()
                    } else {
                        unreachable!()
                    };
                // Constrói a expressão do limit: IntLit se for dígitos, Ident caso contrário.
                let limit_expr = if limit_text.chars().all(|c| c.is_ascii_digit()) {
                    Spanned::new(Expr::IntLit { text: limit_text }, pipe_span)
                } else {
                    Spanned::new(Expr::Ident { name: limit_text }, pipe_span)
                };
                let mut rhs = parse_apply(parser)?;
                // rhs também pode ter `?` postfix.
                while matches!(parser.peek(), Token::Question) {
                    let q_span = parser.advance();
                    let span = rhs.span.cover(q_span);
                    rhs = Spanned::new(Expr::Question(Box::new(rhs)), span);
                }
                let span = lhs.span.cover(rhs.span);
                lhs = Spanned::new(
                    Expr::PipeLimit {
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                        limit: Box::new(limit_expr),
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
                // `tx <! valor` — envio por canal.
                parser.advance(); // consume `<!`
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
                // `rx !> nome` — recebimento de canal.
                // `!>` exige um Ident como destino (binding name).
                parser.advance(); // consume `!>`
                let name = match parser.peek() {
                    Token::Ident(s) => {
                        let n = s.clone();
                        parser.advance();
                        n
                    }
                    _ => {
                        return Err(parser
                            .error("identificador após `!>` (nome do binding de recebimento)"));
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
    parse_apply_impl(parser, false)
}

/// Parse um argumento posicional em modo arity-aware.
/// Igual a `parse_apply` mas Grouping/paren NUNCA coleta args — é sempre valor.
/// `(+ 1 2)` como argumento é valor, não callee que consome mais args.
pub(crate) fn parse_arg(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
    parse_apply_impl(parser, true)
}

fn parse_apply_impl(parser: &mut Parser, as_arg: bool) -> Result<Spanned<Expr>, FrontendError> {
    let callee = parser.parse_expr_post_ascription()?;

    // Literais, construções de statement e keywords de controle de fluxo
    // não são callee — não consomem argumentos.
    if matches!(
        &callee.node,
        Expr::IntLit { .. }
            | Expr::FloatLit { .. }
            | Expr::TextLit { .. }
            | Expr::BytesLit { .. }
            | Expr::Unit
            | Expr::Tuple { .. }
            | Expr::ListLit { .. }
            | Expr::ArrayLit { .. }
            | Expr::SetLit { .. }
            | Expr::DictLit { .. }
            | Expr::RangeLit { .. }
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

    // Grouping como argumento é sempre valor — nunca coleta args.
    // `(+ 1 2) 3` — o Grouping `(+ 1 2)` é o 1º arg de algo externo, `3` pertence
    // ao callee externo, não ao Grouping.
    if as_arg && matches!(callee.node, Expr::Grouping { .. }) {
        return Ok(callee);
    }

    // ── Arity-aware branch ──────────────────────────────────────
    // Se o parser tem tabela de aridades e o callee é `Ident(name)` com
    // aridade conhecida, coleta exatamente N argumentos posicionais — cada
    // um via `parse_arg` recursivo (permite sub-aplicações como
    // `+ 5 * 2 2` → arg2 = `Apply(*, [2, 2])`). Após coletar N args, se o
    // próximo token `can_start_expr()` e não é `StmtSep`/`Eof` → erro.
    if let Some(ref arities) = parser.arities
        && let Expr::Ident { ref name } = callee.node
        && let Some(&arity) = arities.get(name)
    {
        let mut args = Vec::with_capacity(arity);
        for i in 0..arity {
            if !parser.can_start_expr() {
                // Se coletamos 0 args e o próximo token não inicia
                // expr, o Ident é valor (referência), não aplicação.
                // Ex: `(f)` em `map (f) [1 2 3]` — f tem aridade 1
                // mas `)` não inicia expr → f é valor.
                if i == 0 {
                    return Ok(callee);
                }
                return Err(FrontendError::UnexpectedToken {
                    expected: format!(
                        "argumento #{} para `{}` (aridade padrão {})",
                        i + 1,
                        name,
                        arity
                    ),
                    found: parser.peek().to_string(),
                    span: kata_diagnostics::MietteSpan(parser.peek_span()),
                });
            }
            args.push(parse_arg(parser)?);
        }
        // Após coletar N args, verificar excesso posicional.
        if parser.can_start_expr() && !matches!(parser.peek(), Token::StmtSep | Token::Eof) {
            let span = callee.span.cover(parser.peek_span());
            return Err(FrontendError::UnexpectedToken {
                expected: format!(
                    "`{}` tem aridade padrão {} — excesso de argumentos posicionais. \
                         Use `{}{{...}}` para aridade diferente ou separe com quebra de linha.",
                    name, arity, name
                ),
                found: parser.peek().to_string(),
                span: kata_diagnostics::MietteSpan(span),
            });
        }
        if args.is_empty() {
            return Ok(callee);
        }
        let span = callee.span.cover(args.last().expect("non-empty args").span);
        return Ok(Spanned::new(
            Expr::Apply {
                callee: Box::new(callee),
                args,
            },
            span,
        ));
    }

    // ── Greedy mode (fallback) ──────────────────────────────────
    // Greedy atoms ativo quando:
    // - arities é None (modo original), ou
    // - as_arg é false (top-level: construtores, funções sem aridade
    //   conhecida, etc. — coletam args greedy como antes)
    // Quando as_arg é true e arities é Some, o átomo é valor — não coleta
    // args. Isso evita que `+ a b` dentro de um lambda trate `a` como
    // callee greedy e consuma `b`, ou que `+ _ n` trate `_` como callee.
    if parser.arities.is_some() && as_arg {
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
