//! Expressions — atoms, application, let, paren, type ascription.

use kata_ast::{DotIndex, Expr, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    /// Determine if the current token can start an expression.
    pub(crate) fn can_start_expr(&self) -> bool {
        matches!(
            self.peek(),
            Token::IntLit(_)
                | Token::FloatLit(_)
                | Token::TextLit(_)
                | Token::Ident(_)
                | Token::LParen
                | Token::Let
                | Token::Var
                | Token::Lambda
                | Token::Match
                | Token::Return
                | Token::Loop
                | Token::Break
                | Token::Continue
        )
    }

    pub(crate) fn parse_let(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Let, "`let`")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("binding name after `let`")),
        };
        self.expect(&Token::BindAssign, "`:=`")?;
        let value = parse_expr(self)?;
        // Cover span
        let span = start.cover(value.span);
        Ok(Spanned::new(
            Expr::Let {
                name,
                value: Box::new(value),
            },
            span,
        ))
    }

    /// Parse a single expression atom (no application).
    pub(crate) fn parse_expr_atom(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        match self.peek().clone() {
            Token::IntLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::IntLit { text: s }, start))
            }
            Token::FloatLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::FloatLit { text: s }, start))
            }
            Token::TextLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::TextLit { text: s }, start))
            }
            Token::Let => self.parse_let(),
            Token::Var => {
                if !self.in_action_body {
                    return Err(self.error("`var` fora de Action (var só existe em Actions)"));
                }
                self.parse_var()
            }
            Token::Lambda => self.parse_lambda(),
            Token::Match => self.parse_match(),
            Token::Return => {
                if !self.in_action_body {
                    return Err(self.error("`return` fora de Action (return só existe em Actions)"));
                }
                self.parse_return()
            }
            Token::Loop => {
                if !self.in_action_body {
                    return Err(self.error("`loop` fora de Action (loop só existe em Actions)"));
                }
                self.parse_loop()
            }
            Token::Break => {
                if !self.in_action_body {
                    return Err(self.error("`break` fora de Action (break só existe em Actions)"));
                }
                self.advance();
                Ok(Spanned::new(Expr::Break, start))
            }
            Token::Continue => {
                if !self.in_action_body {
                    return Err(
                        self.error("`continue` fora de Action (continue só existe em Actions)")
                    );
                }
                self.advance();
                Ok(Spanned::new(Expr::Continue, start))
            }
            Token::LParen => self.parse_paren_expr(),
            Token::Ident(name) => {
                self.advance();
                // `_` em posição de expressão → Hole (currying).
                // Em posição de pattern, o parser produz Pattern::Wildcard
                // (disambiguação no parser, não no typeck).
                if name == "_" {
                    return Ok(Spanned::new(Expr::Hole, start));
                }
                // Check for VariantQual: Ident :: Ident
                // But `::` is also TypeAscription (expr::Type).
                // Disambiguation: if the Ident after `::` is a known type name vs variant...
                // The parser can't know. The PRD says VariantQual is `Enum::Variant`.
                // TypeAscription is `expr::Type`.
                // Heuristic: `Ident::Ident` where both are capitalized → VariantQual
                // Actually, the PRD says "The parser doesn't know if Boolean is type or
                // module — produces VariantQual and typeck resolves."
                // But `3.14::Rational` is TypeAscription(FloatLit, Named(Rational)).
                // So: if left side is a literal → TypeAscription
                // If left side is an Ident and we see `::` followed by Ident → VariantQual
                // If left side is an Ident and we see `::` followed by something that's a
                // type expression (not a simple Ident that could be a variant) → ambiguous
                //
                // For Fio 1: the only use of `::` with an Ident on the left is VariantQual.
                // TypeAscription with `::` is only for literals (`3.14::Rational`).
                // So: Ident :: Ident → VariantQual
                //     Literal :: Type → TypeAscription
                if matches!(self.peek(), Token::DoubleColon)
                    && let Some(next) = self.tokens.get(self.pos + 1)
                    && let Token::Ident(variant) = &next.token
                {
                    let variant = variant.clone();
                    self.advance(); // consume ::
                    self.advance(); // consume variant Ident
                    let span = start.cover(self.tokens[self.pos - 1].span);
                    return Ok(Spanned::new(
                        Expr::VariantQual {
                            enum_name: name,
                            variant,
                        },
                        span,
                    ));
                }
                // Check for ActionCall: Ident ! (tuple)
                // `echo!("msg")` → ActionCall { callee: "echo", args: Tuple }
                if matches!(self.peek(), Token::Bang) {
                    self.advance(); // consume !
                    // `!` deve ser seguido de `(` para a tupla de argumentos.
                    let args = self.parse_paren_expr()?;
                    let span = start.cover(args.span);
                    return Ok(Spanned::new(
                        Expr::ActionCall {
                            callee: name,
                            args: Box::new(args),
                        },
                        span,
                    ));
                }
                // Check for Reassign: Ident := expr (sem `let`/`var` prefix).
                // `x := 42` → Reassign { name: "x", value: 42 }.
                if matches!(self.peek(), Token::BindAssign) {
                    self.advance(); // consume :=
                    let value = parse_expr(self)?;
                    let span = start.cover(value.span);
                    return Ok(Spanned::new(
                        Expr::Reassign {
                            name,
                            value: Box::new(value),
                        },
                        span,
                    ));
                }
                Ok(Spanned::new(Expr::Ident { name }, start))
            }
            _ => Err(self.error("expression")),
        }
    }

    /// Parse parenthesized expression: `()`, `(expr)`, `(a, b, c)`, `(a,)`
    pub(crate) fn parse_paren_expr(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::LParen, "`(`")?;

        // `()` = Unit
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            return Ok(Spanned::new(Expr::Unit, start));
        }

        // Parse first expression
        let first = parse_expr(self)?;

        // No comma → grouping
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            let span = start.cover(first.span);
            return Ok(Spanned::new(
                Expr::Grouping {
                    inner: Box::new(first),
                },
                span,
            ));
        }

        // Comma present → tuple
        let mut elements = vec![first];
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            // Trailing comma: `(a,)` → tuple of 1
            if matches!(self.peek(), Token::RParen) {
                break;
            }
            elements.push(parse_expr(self)?);
        }

        self.expect(&Token::RParen, "`)`")?;
        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(Expr::Tuple { elements }, span))
    }

    /// Parse an expression with greedy application.
    pub(crate) fn parse_expr_post_ascription(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let mut atom = self.parse_expr_atom()?;

        // ── Fio 5: DotAccess postfix — `expr.nome`, `expr.0`, `expr.(-1)` ──
        // Loop permite encadeamento: `pessoa.endereco.rua`.
        // Precedência: dot é mais apertado que ascription (`pessoa.nome::Text`
        // = `(pessoa.nome)::Text`).
        loop {
            if !matches!(self.peek(), Token::Dot) {
                break;
            }
            self.advance(); // consume `.`

            let index = match self.peek().clone() {
                Token::Ident(name) => {
                    self.advance();
                    DotIndex::Field(name)
                }
                Token::IntLit(text) => {
                    self.advance();
                    let n: i64 = text
                        .parse()
                        .map_err(|_| self.error("inteiro após `.`"))?;
                    DotIndex::Int(n)
                }
                // `t.(-1)` — índice negativo entre parênteses.
                // `(` após `.` deve conter `[-] IntLit`.
                Token::LParen => {
                    self.advance(); // consume `(`
                    let n: i64 = match self.peek().clone() {
                        Token::IntLit(text) => {
                            self.advance();
                            text.parse()
                                .map_err(|_| self.error("inteiro dentro de `.()`"))?
                        }
                        // `-` é Ident("-") na notação prefixa de Kata.
                        Token::Ident(s) if s == "-" => {
                            self.advance();
                            match self.peek().clone() {
                                Token::IntLit(text) => {
                                    self.advance();
                                    -(text
                                        .parse::<i64>()
                                        .map_err(|_| self.error("inteiro após `-`"))?)
                                }
                                _ => return Err(self.error("inteiro após `-`")),
                            }
                        }
                        _ => return Err(self.error("inteiro ou `-inteiro` dentro de `.()`")),
                    };
                    self.expect(&Token::RParen, "`)`")?;
                    DotIndex::Int(n)
                }
                _ => return Err(self.error("identificador, inteiro ou `(` após `.`")),
            };

            let span = atom.span.cover(self.tokens[self.pos - 1].span);
            atom = Spanned::new(
                Expr::DotAccess {
                    expr: Box::new(atom),
                    index,
                },
                span,
            );
        }

        // Check for TypeAscription: expr::Type
        if matches!(self.peek(), Token::DoubleColon) {
            self.advance(); // consume ::
            let ty = self.parse_type_expr()?;
            let span = atom.span.cover(ty.span);
            return Ok(Spanned::new(
                Expr::TypeAscription {
                    expr: Box::new(atom),
                    ty,
                },
                span,
            ));
        }

        Ok(atom)
    }

    /// Parse an atom or ascription — used for arguments in Apply.
    /// Arguments don't greedily consume more arguments themselves.
    pub(crate) fn parse_expr_atom_or_ascription(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        self.parse_expr_post_ascription()
    }

    /// Parse `var nome := expr` — binding mutável (exclusivo de Actions).
    pub(crate) fn parse_var(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Var, "`var`")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("binding name after `var`")),
        };
        self.expect(&Token::BindAssign, "`:=`")?;
        let value = parse_expr(self)?;
        let span = start.cover(value.span);
        Ok(Spanned::new(
            Expr::Var {
                name,
                value: Box::new(value),
            },
            span,
        ))
    }

    /// Parse `return expr` — early return (exclusivo de Actions).
    pub(crate) fn parse_return(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Return, "`return`")?;
        let value = parse_expr(self)?;
        let span = start.cover(value.span);
        Ok(Spanned::new(Expr::Return(Box::new(value)), span))
    }

    /// Parse `loop` with indented body — laço infinito (exclusivo de Actions).
    pub(crate) fn parse_loop(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Loop, "`loop`")?;
        self.expect(&Token::Indent, "INDENT (loop body)")?;

        let mut body = Vec::new();
        loop {
            while matches!(self.peek(), Token::StmtSep | Token::Semicolon) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            let stmt = parse_expr(self)?;
            body.push(stmt);
        }

        self.expect(&Token::Dedent, "DEDENT (fim do loop body)")?;
        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(Expr::Loop { body }, span))
    }
}

/// Parse an expression with greedy application.
/// Free function — called from declarations and expressions.
///
/// After parsing the application, checks for `|>` (pipe) infix operator.
/// `|>` has lower precedence than application and is left-associative.
/// `a |> b |> c` = `(a |> b) |> c`.
pub(crate) fn parse_expr(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
    let mut lhs = parse_apply(parser)?;

    // `?` postfix — fail-fast operator (Fase 7).
    // `expr ?` → Expr::Question(Box<expr>).
    // Precedência: maior que `|>` (pipe), menor que `::` (ascription).
    // Ou seja: `Result::Ok 42 ?` = `(Result::Ok 42) ?`, e
    // `x ? |> f` = `(x ?) |> f`.
    while matches!(parser.peek(), Token::Question) {
        let q_span = parser.advance(); // consume ?
        let span = lhs.span.cover(q_span);
        lhs = Spanned::new(Expr::Question(Box::new(lhs)), span);
    }

    // `|>` pipe and `|` fallback — same precedence, left-associative.
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
            _ => break,
        }
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
fn parse_apply(parser: &mut Parser) -> Result<Spanned<Expr>, FrontendError> {
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
            | Expr::Unit
            | Expr::Let { .. }
            | Expr::Var { .. }
            | Expr::Reassign { .. }
            | Expr::Match { .. }
            | Expr::Loop { .. }
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
