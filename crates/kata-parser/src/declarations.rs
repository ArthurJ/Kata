//! Declarations — parse_module, directives, sig, data, enum, fields.

use kata_ast::{Directive, Expr, Item, LambdaClause, Module, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::CasingPattern;
use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    pub(crate) fn parse_module(&mut self) -> Result<Module, FrontendError> {
        let mut items: Vec<Spanned<Item>> = Vec::new();

        while !self.at_eof() {
            // Skip leading statement separators
            if matches!(self.peek(), Token::StmtSep) {
                self.advance();
                continue;
            }

            // Collect directives (zero or more @name ... prefixes)
            let directives = self.parse_directives()?;

            // Skip statement separators that may appear after directives
            // (directives end a line, StmtSep appears before the item)
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }

            // Now parse the item
            if matches!(self.peek(), Token::Eof) {
                break;
            }

            let item_start = self.peek_span();

            match self.peek() {
                Token::Data => {
                    let item = self.parse_data_decl(directives)?;
                    items.push(Spanned::new(item, item_start));
                }
                Token::Enum => {
                    let item = self.parse_enum_decl(directives)?;
                    items.push(Spanned::new(item, item_start));
                }
                Token::Alias => {
                    let item = self.parse_alias_decl(directives)?;
                    items.push(Spanned::new(item, item_start));
                }
                Token::Action => {
                    let item = self.parse_action_decl(directives)?;
                    items.push(Spanned::new(item, item_start));
                }
                Token::Interface => {
                    let item = self.parse_interface_decl(directives)?;
                    items.push(Spanned::new(item, item_start));
                }
                Token::Implements => {
                    // Sintaxe antiga `implements IFACE for TYPE` — não deve
                    // aparecer como top-level item. A sintaxe correta é
                    // `TYPE implements IFACE`, despachada no braço `_ =>`.
                    // Se chega aqui, é erro de sintaxe.
                    return Err(self.error("expected type name before `implements`"));
                }
                Token::Import => {
                    let item = self.parse_import_decl()?;
                    items.push(Spanned::new(item, item_start));
                }
                Token::Export => {
                    let item = self.parse_export_decl()?;
                    items.push(Spanned::new(item, item_start));
                }
                Token::Let => {
                    // Top-level let: produce a Let item
                    let expr = self.parse_let()?;
                    // Se há diretiva @comptime, envolver o let em Expr::Comptime.
                    let final_expr = if directives.iter().any(|d| d.name == "comptime") {
                        Spanned::new(
                            Expr::Comptime {
                                expr: Box::new(expr.clone()),
                            },
                            expr.span,
                        )
                    } else {
                        expr.clone()
                    };
                    items.push(Spanned::new(
                        Item::EntryExpr(final_expr.clone()),
                        final_expr.span,
                    ));
                }
                _ => {
                    // Could be a signature (name :: Type...), an implements
                    // decl (Tipo implements IFACE), a refines decl
                    // (TipoRefinado refines IFACE), or an expression.
                    if self.is_implements_start() {
                        let item = self.parse_implements_decl(directives)?;
                        items.push(Spanned::new(item, item_start));
                    } else if self.is_refines_start() {
                        let item = self.parse_refines_decl(directives)?;
                        items.push(Spanned::new(item, item_start));
                    } else if self.is_signature_start() {
                        let item = self.parse_sig(directives)?;
                        items.push(Spanned::new(item, item_start));
                    } else {
                        // Entry expression
                        let expr = parse_expr(self)?;
                        // Consume trailing separators
                        while matches!(self.peek(), Token::StmtSep) {
                            self.advance();
                        }
                        // Se há diretiva @comptime, envolver em Expr::Comptime.
                        let final_expr = if directives.iter().any(|d| d.name == "comptime") {
                            Spanned::new(
                                Expr::Comptime {
                                    expr: Box::new(expr.clone()),
                                },
                                expr.span,
                            )
                        } else {
                            expr.clone()
                        };
                        items.push(Spanned::new(
                            Item::EntryExpr(final_expr.clone()),
                            final_expr.span,
                        ));
                    }
                }
            }
        }

        Ok(Module { items })
    }

    /// Parse module com error recovery de top-level items.
    ///
    /// Igual ao `parse_module`, mas quando um item falha, registra o erro
    /// em `errors` e skipa tokens até o próximo `StmtSep` ou `Eof`, então
    /// continua. Retorna `Ok(Module)` com os items parseados com sucesso
    /// (pode ser vazio se tudo falhou) e `Vec<FrontendError>` com os erros.
    pub(crate) fn parse_module_with_recovery(&mut self) -> (Module, Vec<FrontendError>) {
        let mut items: Vec<Spanned<Item>> = Vec::new();
        let mut errors: Vec<FrontendError> = Vec::new();

        while !self.at_eof() {
            // Skip leading statement separators
            if matches!(self.peek(), Token::StmtSep) {
                self.advance();
                continue;
            }

            // Collect directives (zero or more @name ... prefixes)
            let directives = match self.parse_directives() {
                Ok(d) => d,
                Err(e) => {
                    errors.push(e);
                    self.sync_to_stmt_sep();
                    continue;
                }
            };

            // Skip statement separators that may appear after directives
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }

            if matches!(self.peek(), Token::Eof) {
                break;
            }

            let item_start = self.peek_span();

            match self.peek() {
                Token::Data => match self.parse_data_decl(directives) {
                    Ok(item) => items.push(Spanned::new(item, item_start)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Enum => match self.parse_enum_decl(directives) {
                    Ok(item) => items.push(Spanned::new(item, item_start)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Alias => match self.parse_alias_decl(directives) {
                    Ok(item) => items.push(Spanned::new(item, item_start)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Action => match self.parse_action_decl(directives) {
                    Ok(item) => items.push(Spanned::new(item, item_start)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Interface => match self.parse_interface_decl(directives) {
                    Ok(item) => items.push(Spanned::new(item, item_start)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Implements => {
                    errors.push(self.error("expected type name before `implements`"));
                    self.sync_to_stmt_sep();
                }
                Token::Import => match self.parse_import_decl() {
                    Ok(item) => items.push(Spanned::new(item, item_start)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Export => match self.parse_export_decl() {
                    Ok(item) => items.push(Spanned::new(item, item_start)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Let => match self.parse_let() {
                    Ok(expr) => {
                        // Se há diretiva @comptime, envolver em Expr::Comptime.
                        let final_expr = if directives.iter().any(|d| d.name == "comptime") {
                            Spanned::new(
                                Expr::Comptime {
                                    expr: Box::new(expr.clone()),
                                },
                                expr.span,
                            )
                        } else {
                            expr.clone()
                        };
                        items.push(Spanned::new(
                            Item::EntryExpr(final_expr.clone()),
                            final_expr.span,
                        ));
                    }
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                _ => {
                    // Signature, implements, refines, ou expression
                    if self.is_implements_start() {
                        match self.parse_implements_decl(directives) {
                            Ok(item) => items.push(Spanned::new(item, item_start)),
                            Err(e) => {
                                errors.push(e);
                                self.sync_to_stmt_sep();
                            }
                        }
                    } else if self.is_refines_start() {
                        match self.parse_refines_decl(directives) {
                            Ok(item) => items.push(Spanned::new(item, item_start)),
                            Err(e) => {
                                errors.push(e);
                                self.sync_to_stmt_sep();
                            }
                        }
                    } else if self.is_signature_start() {
                        match self.parse_sig(directives) {
                            Ok(item) => items.push(Spanned::new(item, item_start)),
                            Err(e) => {
                                errors.push(e);
                                self.sync_to_stmt_sep();
                            }
                        }
                    } else {
                        match parse_expr(self) {
                            Ok(expr) => {
                                while matches!(self.peek(), Token::StmtSep) {
                                    self.advance();
                                }
                                // Se há diretiva @comptime, envolver em Expr::Comptime.
                                let final_expr = if directives.iter().any(|d| d.name == "comptime")
                                {
                                    Spanned::new(
                                        Expr::Comptime {
                                            expr: Box::new(expr.clone()),
                                        },
                                        expr.span,
                                    )
                                } else {
                                    expr.clone()
                                };
                                items.push(Spanned::new(
                                    Item::EntryExpr(final_expr.clone()),
                                    final_expr.span,
                                ));
                            }
                            Err(e) => {
                                errors.push(e);
                                self.sync_to_stmt_sep();
                            }
                        }
                    }
                }
            }
        }

        (Module { items }, errors)
    }

    /// Sincroniza: avança tokens até o próximo `StmtSep` ou `Eof`.
    fn sync_to_stmt_sep(&mut self) {
        while !self.at_eof() {
            match self.peek() {
                Token::StmtSep => {
                    self.advance();
                    break;
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    /// Check if the current position starts an implements decl:
    /// `Tipo implements IFACE` or `Tipo::(params) implements IFACE`.
    /// Looks ahead past optional `::(params)` to find `implements`.
    fn is_implements_start(&self) -> bool {
        if !matches!(self.peek(), Token::Ident(_)) {
            return false;
        }
        let mut lookahead = self.pos + 1;
        // Skip optional `::(params)` or `::Param` — type params of the tipo.
        if let Some(t) = self.tokens.get(lookahead)
            && matches!(t.token, Token::DoubleColon)
        {
            lookahead += 1;
            if let Some(t2) = self.tokens.get(lookahead)
                && matches!(t2.token, Token::LParen)
            {
                let mut depth = 1;
                lookahead += 1;
                while lookahead < self.tokens.len() && depth > 0 {
                    match &self.tokens[lookahead].token {
                        Token::LParen => depth += 1,
                        Token::RParen => depth -= 1,
                        _ => {}
                    }
                    lookahead += 1;
                }
            } else {
                // `::Param` — single param without parens, skip 1 token.
                lookahead += 1;
            }
        }
        self.tokens
            .get(lookahead)
            .map(|t| matches!(t.token, Token::Implements))
            .unwrap_or(false)
    }

    /// Check if the current position starts a refines decl:
    /// `TipoRefinado refines IFACE`.
    /// Lookahead: Ident followed by `refines`.
    fn is_refines_start(&self) -> bool {
        if !matches!(self.peek(), Token::Ident(_)) {
            return false;
        }
        self.tokens
            .get(self.pos + 1)
            .map(|t| matches!(t.token, Token::Refines))
            .unwrap_or(false)
    }

    /// Check if the current position starts a signature: `name :: Type... => RetType`
    /// Distinguishes from VariantQual (`Enum::Variant`) by scanning ahead for `=>`.
    fn is_signature_start(&self) -> bool {
        if let Token::Ident(_) = self.peek() {
            if let Some(next) = self.tokens.get(self.pos + 1) {
                if !matches!(next.token, Token::DoubleColon) {
                    return false;
                }
            } else {
                return false;
            }
            // We have Ident :: ... — scan ahead for `=>` (signature) vs not (VariantQual)
            // Skip past the `::` and scan tokens until we hit `=>`, Eof, or a stopper.
            let mut lookahead = self.pos + 2;
            while lookahead < self.tokens.len() {
                match &self.tokens[lookahead].token {
                    Token::FatArrow => return true,
                    // These tokens can't appear in a type param list, so if we
                    // see them before `=>`, it's not a signature.
                    Token::Eof | Token::StmtSep | Token::Indent | Token::Dedent => return false,
                    _ => lookahead += 1,
                }
            }
            false
        } else {
            false
        }
    }

    // ── Items ────────────────────────────────────────────────────

    fn parse_sig(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        // name :: T1 T2 ... => TRet
        let name = match self.peek() {
            Token::Ident(s) => {
                let span = self.peek_span();
                let n = s.clone();
                self.advance();
                // Validar casing apenas para nomes alfabéticos (não símbolos como +, -, *)
                if n.chars().next().is_some_and(|c| c.is_alphabetic()) {
                    self.validate_name(&n, CasingPattern::SnakeCase, span)?;
                }
                n
            }
            _ => return Err(self.error("signature name")),
        };
        self.expect(&Token::DoubleColon, "`::`")?;

        // Parse type params until `=>`
        let mut params = Vec::new();
        let mut param_names = Vec::new();
        while !matches!(self.peek(), Token::FatArrow | Token::Eof) {
            params.push(self.parse_type_expr()?);
            param_names.push(None);
        }

        self.expect(&Token::FatArrow, "`=>`")?;
        let ret = self.parse_type_expr()?;

        // Cláusulas lambda após assinatura (função nomeada com corpo Kata).
        // Lambda no mesmo nível da assinatura: se o próximo token é `lambda`,
        // parsear cláusulas. Se não, body = None (FFI — corpo suprido por @ffi).
        // Consumir StmtSep antes de checar (newline entre assinatura e lambda).
        while matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }
        let body = if matches!(self.peek(), Token::Lambda) {
            Some(self.parse_sig_clauses()?)
        } else {
            None
        };

        // Consume trailing StmtSep if present
        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(Item::Sig {
            name,
            params,
            param_names,
            ret,
            directives,
            body,
        })
    }

    /// Parse cláusulas lambda no mesmo nível da assinatura.
    ///
    /// ```text
    /// fat :: Int Int => Int
    /// lambda 0 acc: acc
    /// lambda n acc: fat (- n 1) (* n acc)
    /// ```
    ///
    /// Terminação: consome lambdas até encontrar um token que não seja
    /// `Lambda` nem `StmtSep` (nova assinatura, action, diretiva, EOF, etc.).
    pub(crate) fn parse_sig_clauses(
        &mut self,
    ) -> Result<Vec<Spanned<LambdaClause>>, FrontendError> {
        let mut clauses = Vec::new();
        loop {
            // Skip StmtSep entre cláusulas
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            // Fim: EOF, Dedent (contexto implements), ou token não-lambda
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            if !matches!(self.peek(), Token::Lambda) {
                break;
            }

            let clause_start = self.peek_span();
            let clause = self.parse_lambda_clause()?;
            let span = clause_start.cover(clause.body.span);
            clauses.push(Spanned::new(clause, span));
        }

        Ok(clauses)
    }

    /// Parse uma cláusula lambda: `lambda <patterns>: <body>`.
    /// Body é expressão única (sem guards, sem with).
    /// Body pode ser bloco indentado com guards + with.
    fn parse_lambda_clause(&mut self) -> Result<LambdaClause, FrontendError> {
        self.expect(&Token::Lambda, "`lambda`")?;

        // Parse patterns (1 ou mais, separados por espaço)
        let patterns = self.parse_patterns()?;

        // Expect `:`
        self.expect(&Token::Colon, "`:` após patterns da cláusula")?;

        // Body é expressão única (sem guards).
        // Se há INDENT após `:`, é bloco com guards + with.
        if matches!(self.peek(), Token::Indent) {
            // Bloco indentado — guards + with.
            // Reusar parse_lambda_body_block do expressions module.
            // Precisamos extrair guards e with_bindings do Lambda retornado.
            let clause_start = self.peek_span();
            let lambda_expr = self.parse_lambda_body_block(clause_start, patterns)?;
            // Desempacotar o Lambda para extrair guards e with_bindings
            match lambda_expr.node {
                Expr::Lambda {
                    patterns,
                    body,
                    guards,
                    with_bindings,
                } => {
                    // Consume trailing StmtSep
                    if matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    return Ok(LambdaClause {
                        patterns,
                        body: *body,
                        guards,
                        with_bindings,
                    });
                }
                other => panic!("parse_lambda_body_block returned {other:?}, expected Lambda"),
            }
        }

        let body = parse_expr(self)?;

        // Consume trailing StmtSep
        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(LambdaClause {
            patterns,
            body,
            guards: Vec::new(),
            with_bindings: Vec::new(),
        })
    }
}
