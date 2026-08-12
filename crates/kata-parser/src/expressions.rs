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
                | Token::BytesLit(_)
                | Token::Ident(_)
                | Token::LParen
                | Token::LBracket
                | Token::LBrace
                | Token::Let
                | Token::Var
                | Token::Lambda
                | Token::Match
                | Token::Return
                | Token::Loop
                | Token::Break
                | Token::Continue
                | Token::For
                | Token::Select
                | Token::At
        )
    }

    pub(crate) fn parse_let(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Let, "`let`")?;

        // `let (x, y, ...) := expr` — destructuring de tupla.
        if matches!(self.peek(), Token::LParen) {
            self.advance(); // consume (
            let mut names: Vec<String> = Vec::new();
            // `()` — tupla vazia
            if matches!(self.peek(), Token::RParen) {
                self.advance();
            } else {
                loop {
                    let name = match self.peek() {
                        Token::Ident(s) => {
                            let n = s.clone();
                            self.advance();
                            n
                        }
                        _ => return Err(self.error("binding name or `_` in destructuring")),
                    };
                    names.push(name);
                    if matches!(self.peek(), Token::Comma) {
                        self.advance();
                        // trailing comma: `(x, y,)`
                        if matches!(self.peek(), Token::RParen) {
                            self.advance();
                            break;
                        }
                    } else {
                        self.expect(&Token::RParen, "`)` para fechar destructuring")?;
                        break;
                    }
                }
            }
            self.expect(&Token::BindAssign, "`:=`")?;
            let value = parse_expr(self)?;
            let span = start.cover(value.span);
            return Ok(Spanned::new(
                Expr::LetDestruct {
                    names,
                    value: Box::new(value),
                },
                span,
            ));
        }

        // `let name := expr` — binding simples.
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
            Token::BytesLit(bytes) => {
                self.advance();
                Ok(Spanned::new(Expr::BytesLit { bytes }, start))
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
            Token::For => {
                if !self.in_action_body {
                    return Err(self.error("`for` fora de Action (for só existe em Actions)"));
                }
                self.parse_for_in()
            }
            Token::Select => {
                if !self.in_action_body {
                    return Err(self.error("`select` fora de Action (select só existe em Actions)"));
                }
                self.parse_select()
            }
            Token::Type => {
                self.advance(); // consume `type`
                // `type!` — introspecção compile-time
                self.expect(&Token::Bang, "`!` após `type`")?;
                let inner = self.parse_paren_expr()?;
                let span = start.cover(inner.span);
                Ok(Spanned::new(
                    Expr::TypeOf {
                        expr: Box::new(inner),
                    },
                    span,
                ))
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
            Token::LBracket => self.parse_list_or_range(),
            Token::LBrace => self.parse_brace_lit(),
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
                // The only use of `::` with an Ident on the left is VariantQual.
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
                            module_path: None,
                        },
                        span,
                    ));
                }
                // Check for qualified VariantQual: Ident (. Ident)+ :: Ident
                // e.g. `core.Result::Err` → module_path = ["core"], enum_name = "Result"
                // Lookahead não-destrutivo: se não encontrar `::` no final,
                // restaura a posição e deixa DotAccess (post_ascription) lidar.
                if matches!(self.peek(), Token::Dot) {
                    let saved_pos = self.pos;
                    let mut path = vec![name.clone()];
                    loop {
                        if !matches!(self.peek(), Token::Dot) {
                            break;
                        }
                        self.advance(); // consume .
                        match self.peek() {
                            Token::Ident(s) => {
                                path.push(s.clone());
                                self.advance(); // consume Ident
                            }
                            _ => break, // not Ident after . — not VariantQual
                        }
                        // Check for :: Ident after each path component
                        if matches!(self.peek(), Token::DoubleColon)
                            && let Some(next) = self.tokens.get(self.pos + 1)
                            && let Token::Ident(variant) = &next.token
                        {
                            let variant = variant.clone();
                            self.advance(); // consume ::
                            self.advance(); // consume variant Ident
                            // Last element of path is enum_name, rest is module_path
                            let enum_name = path.pop().expect("path tem >=2 elementos");
                            let module_path = if path.is_empty() { None } else { Some(path) };
                            let span = start.cover(self.tokens[self.pos - 1].span);
                            return Ok(Spanned::new(
                                Expr::VariantQual {
                                    enum_name,
                                    variant,
                                    module_path,
                                },
                                span,
                            ));
                        }
                    }
                    // Not a qualified VariantQual — restore position for DotAccess
                    self.pos = saved_pos;
                }
                // Check for ActionCall: Ident ! (tuple) or Ident ! {dict}
                // `echo!("msg")` → ActionCall { callee: "echo", args: Tuple }
                // `g!{"b": 2 "a": 1}` → ActionCall { callee: "g", args: DictLit }
                if matches!(self.peek(), Token::Bang) {
                    self.advance(); // consume !
                    // `!` pode ser seguido de `(` (tupla posicional) ou `{` (Dict nomeado).
                    let args = if matches!(self.peek(), Token::LBrace) {
                        self.parse_brace_lit()?
                    } else {
                        self.parse_paren_expr()?
                    };
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
            Token::At => {
                // `@comptime` foi removido da linguagem (PRD-constant Fase 5).
                // Usar `constant` para declarações de módulo comptime.
                self.advance(); // consume `@`
                match self.peek() {
                    Token::Ident(s) if s == "comptime" => {
                        Err(self.error(
                            "`@comptime` foi removido. Use `constant` para constantes de módulo, ou remova `@comptime` — o fold automático otimiza chamadas puras com args literais.",
                        ))
                    }
                    _ => Err(self.error("diretiva desconhecida após `@`")),
                }
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

        // ── DotAccess postfix — `expr.nome`, `expr.0`, `expr.(-1)` ──
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
                    let n: i64 = text.parse().map_err(|_| self.error("inteiro após `.`"))?;
                    DotIndex::Int(n)
                }
                // `t.(-1)` — índice negativo entre parênteses.
                // `(` após `.` pode conter:
                //   - `[-] IntLit` → DotIndex::Int (indexing numérico)
                Token::LParen => {
                    self.advance(); // consume `(`
                    match self.peek().clone() {
                        Token::IntLit(text) => {
                            self.advance();
                            let n: i64 = text
                                .parse()
                                .map_err(|_| self.error("inteiro dentro de `.()`"))?;
                            self.expect(&Token::RParen, "`)`")?;
                            DotIndex::Int(n)
                        }
                        Token::Ident(s) if s == "-" => {
                            self.advance();
                            match self.peek().clone() {
                                Token::IntLit(text) => {
                                    self.advance();
                                    let n = -(text
                                        .parse::<i64>()
                                        .map_err(|_| self.error("inteiro após `-`"))?);
                                    self.expect(&Token::RParen, "`)`")?;
                                    DotIndex::Int(n)
                                }
                                _ => return Err(self.error("inteiro após `-`")),
                            }
                        }
                        _ => return Err(self.error("inteiro ou `-inteiro` dentro de `.()`")),
                    }
                }
                // `expr.[start..end]` — slice access (DotIndex::Range).
                // `[` após `.` abre um range slice.
                Token::LBracket => {
                    self.advance(); // consume `[`
                    let start = parse_expr(self)?;
                    // `..` separa start e end (exclusive). `..=` é inclusivo.
                    let inclusive = match self.peek() {
                        Token::DotDot => {
                            self.advance(); // consume `..`
                            false
                        }
                        Token::DotDotEq => {
                            self.advance(); // consume `..=`
                            true
                        }
                        _ => return Err(self.error("`..` ou `..=` após start do slice")),
                    };
                    let end = parse_expr(self)?;
                    self.expect(&Token::RBracket, "`]`")?;
                    DotIndex::Range {
                        start: Box::new(start),
                        end: Box::new(end),
                        inclusive,
                    }
                }
                _ => return Err(self.error("identificador, inteiro, `(` ou `[` após `.`")),
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
}

// Re-export `parse_expr` from `expr_apply` so existing imports
// (`use crate::expressions::parse_expr`) continue to work.
pub(crate) use crate::expr_apply::parse_expr;
