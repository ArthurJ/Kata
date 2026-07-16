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
            Token::For => {
                if !self.in_action_body {
                    return Err(self.error("`for` fora de Action (for só existe em Actions)"));
                }
                self.parse_for_in()
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
            Token::LBrace => self.parse_array_lit(),
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
                    let n: i64 = text.parse().map_err(|_| self.error("inteiro após `.`"))?;
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

    /// Parse `[1 2 3]` (ListLit) ou `[a..s..b]` / `[a..s..=b]` (RangeLit).
    ///
    /// Após `[`, parseia o primeiro elemento. Se o próximo token é `..` ou
    /// `..=`, é Range: parseia step, segundo `..`/`..=`, end. Caso contrário,
    /// coleta elementos restantes para ListLit. `[]` = lista vazia.
    pub(crate) fn parse_list_or_range(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::LBracket, "`[`")?;

        // `[]` — lista vazia
        if matches!(self.peek(), Token::RBracket) {
            self.advance();
            let span = start.cover(self.tokens[self.pos - 1].span);
            return Ok(Spanned::new(Expr::ListLit { elements: vec![] }, span));
        }

        // Parseia primeiro elemento
        let first = parse_expr(self)?;

        // Se vê `..` ou `..=`, é Range: `[start..step..end]` ou `[start..step..=end]`
        // O primeiro `..` é sempre exclusive (separa start de step).
        // Se for `..=` como primeiro separador, é erro de sintaxe.
        match self.peek() {
            Token::DotDot => return self.parse_range_rest(first, start),
            Token::DotDotEq => {
                return Err(self
                    .error("`..` (não `..=`) após start do range — o passo é separado por `..`"));
            }
            _ => {}
        }

        // Caso contrário, é ListLit — coleta elementos restantes
        let mut elements = vec![first];
        while !matches!(self.peek(), Token::RBracket) {
            if matches!(self.peek(), Token::Eof) {
                return Err(self.error("`]` para fechar lista"));
            }
            elements.push(parse_expr(self)?);
        }
        self.expect(&Token::RBracket, "`]`")?;
        let span = start.cover(self.tokens[self.pos - 1].span);
        Ok(Spanned::new(Expr::ListLit { elements }, span))
    }

    /// Parseia o resto de um RangeLit após `start ..` ou `start ..=`.
    /// Já consumiu `start` e está posicionado em `..` ou `..=`.
    pub(crate) fn parse_range_rest(
        &mut self,
        start: Spanned<Expr>,
        bracket_span: kata_ast::Span,
    ) -> Result<Spanned<Expr>, FrontendError> {
        // Consome primeiro `..` (sempre exclusive — o step não usa `..=`)
        self.expect(&Token::DotDot, "`..` após start do range")?;

        // Parseia step
        let step = parse_expr(self)?;

        // Segundo separador: `..` (exclusive) ou `..=` (inclusive)
        let inclusive = match self.peek() {
            Token::DotDot => {
                self.advance();
                false
            }
            Token::DotDotEq => {
                self.advance();
                true
            }
            _ => return Err(self.error("`..` ou `..=` após step do range")),
        };

        // Parseia end
        let end = parse_expr(self)?;

        self.expect(&Token::RBracket, "`]` para fechar range")?;
        let span = bracket_span.cover(self.tokens[self.pos - 1].span);
        Ok(Spanned::new(
            Expr::RangeLit {
                start: Box::new(start),
                step: Box::new(step),
                end: Box::new(end),
                inclusive,
            },
            span,
        ))
    }

    /// Parse `{1 2 3}` (ArrayLit). `{}` = array vazio.
    pub(crate) fn parse_array_lit(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::LBrace, "`{`")?;

        // `{}` — array vazio
        if matches!(self.peek(), Token::RBrace) {
            self.advance();
            let span = start.cover(self.tokens[self.pos - 1].span);
            return Ok(Spanned::new(Expr::ArrayLit { elements: vec![] }, span));
        }

        let mut elements = vec![parse_expr(self)?];
        while !matches!(self.peek(), Token::RBrace) {
            if matches!(self.peek(), Token::Eof) {
                return Err(self.error("`}` para fechar array"));
            }
            elements.push(parse_expr(self)?);
        }
        self.expect(&Token::RBrace, "`}`")?;
        let span = start.cover(self.tokens[self.pos - 1].span);
        Ok(Spanned::new(Expr::ArrayLit { elements }, span))
    }

    /// Parse `for x in colecao` com body indentado (exclusivo de Actions).
    /// Como `loop`: consome INDENT, coleta statements até DEDENT.
    pub(crate) fn parse_for_in(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::For, "`for`")?;

        // `for` deve ser seguido de um identificador (variável de iteração)
        let var_name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("identificador após `for`")),
        };

        // `in` separa a variável da coleção
        self.expect(&Token::In, "`in` após variável do for")?;

        // Parseia a expressão iterável
        let iterable = parse_expr(self)?;

        // Body indentado
        self.expect(&Token::Indent, "INDENT (for body)")?;

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

        self.expect(&Token::Dedent, "DEDENT (fim do for body)")?;
        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(
            Expr::ForIn {
                var_name,
                iterable: Box::new(iterable),
                body,
            },
            span,
        ))
    }
}

// Re-export `parse_expr` from `expr_apply` so existing imports
// (`use crate::expressions::parse_expr`) continue to work.
pub(crate) use crate::expr_apply::parse_expr;
