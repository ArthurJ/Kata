//! Declarations — parse_module, directives, sig, data, enum, fields.

use kata_ast::{
    Directive, DirectiveArg, DirectiveValue, FieldDecl, Item, LambdaClause, Module, Spanned, Token,
    VariantDecl,
};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    pub fn parse_module(&mut self) -> Result<Module, FrontendError> {
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
                Token::Let => {
                    // Top-level let: produce a Let item
                    let expr = self.parse_let()?;
                    items.push(Spanned::new(Item::EntryExpr(expr.clone()), expr.span));
                    // Hmm, actually `let` at top-level should be an Item but
                    // the AST has no Let variant in Item. We wrap it as EntryExpr(Let(...)).
                    // The task says "let ... can appear at top-level too (becomes a Let
                    // binding in the item list)". Since Item has no Let variant, we use
                    // EntryExpr(Let). This is a design limitation of the current AST.
                    // Actually, let's keep it as is — EntryExpr wrapping the Let expr.
                }
                _ => {
                    // Could be a signature (name :: Type ...) or an expression.
                    // Check: if next is Ident followed by DoubleColon, it's a Sig.
                    if self.is_signature_start() {
                        let item = self.parse_sig(directives)?;
                        items.push(Spanned::new(item, item_start));
                    } else {
                        // Entry expression
                        let expr = parse_expr(self)?;
                        // Consume trailing separators
                        while matches!(self.peek(), Token::StmtSep) {
                            self.advance();
                        }
                        items.push(Spanned::new(Item::EntryExpr(expr.clone()), expr.span));
                    }
                }
            }
        }

        Ok(Module { items })
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

    // ── Directives ────────────────────────────────────────────────

    fn parse_directives(&mut self) -> Result<Vec<Directive>, FrontendError> {
        let mut directives = Vec::new();
        loop {
            // Skip statement separators between stacked directives
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if !matches!(self.peek(), Token::At) {
                break;
            }
            let at_span = self.advance(); // consume @
            // Expect an identifier (directive name)
            let name = match self.peek() {
                Token::Ident(s) => {
                    let name = s.clone();
                    self.advance();
                    name
                }
                _ => {
                    return Err(self.error("directive name after @"));
                }
            };
            let args = self.parse_directive_args()?;
            directives.push(Directive {
                name,
                args,
                span: at_span,
            });
        }
        Ok(directives)
    }

    fn parse_directive_args(&mut self) -> Result<Vec<DirectiveArg>, FrontendError> {
        let mut args = Vec::new();
        // @name("arg") — parenthesized positional args
        // @name{key: value} — braced named args
        match self.peek() {
            Token::LParen => {
                self.advance(); // consume (
                // Empty parens: no args
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    return Ok(args);
                }
                loop {
                    let arg = self.parse_one_directive_arg()?;
                    args.push(arg);
                    match self.peek() {
                        Token::Comma => {
                            self.advance();
                        }
                        Token::RParen => {
                            self.advance();
                            break;
                        }
                        _ => return Err(self.error("`,` or `)`")),
                    }
                }
            }
            Token::LBrace => {
                self.advance(); // consume {
                if matches!(self.peek(), Token::RBrace) {
                    self.advance();
                    return Ok(args);
                }
                loop {
                    // key : value
                    let key = match self.peek() {
                        Token::Ident(s) => {
                            let k = s.clone();
                            self.advance();
                            k
                        }
                        _ => return Err(self.error("directive key (identifier)")),
                    };
                    self.expect(&Token::Colon, "`:`")?;
                    let value = self.parse_directive_value()?;
                    args.push(DirectiveArg::Named { key, value });
                    match self.peek() {
                        Token::Comma => {
                            self.advance();
                        }
                        Token::RBrace => {
                            self.advance();
                            break;
                        }
                        _ => return Err(self.error("`,` or `}`")),
                    }
                }
            }
            _ => {}
        }
        Ok(args)
    }

    fn parse_one_directive_arg(&mut self) -> Result<DirectiveArg, FrontendError> {
        match self.peek() {
            Token::TextLit(s) => {
                let val = s.clone();
                self.advance();
                Ok(DirectiveArg::Str(val))
            }
            Token::IntLit(s) => {
                let val: i64 = s.parse().map_err(|_| self.error("integer literal"))?;
                self.advance();
                Ok(DirectiveArg::Int(val))
            }
            _ => Err(self.error("string or integer argument")),
        }
    }

    fn parse_directive_value(&mut self) -> Result<DirectiveValue, FrontendError> {
        match self.peek() {
            Token::TextLit(s) => {
                let val = s.clone();
                self.advance();
                Ok(DirectiveValue::Str(val))
            }
            Token::IntLit(s) => {
                let val: i64 = s.parse().map_err(|_| self.error("integer literal"))?;
                self.advance();
                Ok(DirectiveValue::Int(val))
            }
            _ => Err(self.error("string or integer value")),
        }
    }

    // ── Items ────────────────────────────────────────────────────

    fn parse_sig(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        // name :: T1 T2 ... => TRet
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("signature name")),
        };
        self.expect(&Token::DoubleColon, "`::`")?;

        // Parse type params until `=>`
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::FatArrow | Token::Eof) {
            params.push(self.parse_type_expr()?);
        }

        self.expect(&Token::FatArrow, "`=>`")?;
        let ret = self.parse_type_expr()?;

        // Fio 2: cláusulas lambda após assinatura (função nomeada com corpo Kata).
        // Se há INDENT seguido de Lambda, parsear cláusulas.
        // Se não, body = None (FFI — corpo suprido por @ffi).
        let body = if matches!(self.peek(), Token::Indent) {
            // Verifica se o primeiro token após INDENT é `lambda`
            if let Some(next) = self.tokens.get(self.pos + 1) {
                if matches!(next.token, Token::Lambda) {
                    Some(self.parse_sig_clauses()?)
                } else {
                    None
                }
            } else {
                None
            }
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
            ret,
            directives,
            body,
        })
    }

    /// Parse cláusulas lambda indentadas após uma assinatura.
    ///
    /// ```text
    /// fat :: Int Int => Int
    ///     lambda 0 acc: acc
    ///     lambda n acc: fat (- n 1) (* n acc)
    /// ```
    fn parse_sig_clauses(&mut self) -> Result<Vec<Spanned<LambdaClause>>, FrontendError> {
        self.expect(&Token::Indent, "INDENT (cláusulas lambda)")?;

        let mut clauses = Vec::new();
        loop {
            // Skip StmtSep entre cláusulas
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }

            let clause_start = self.peek_span();
            let clause = self.parse_lambda_clause()?;
            let span = clause_start.cover(clause.body.span);
            clauses.push(Spanned::new(clause, span));
        }

        self.expect(&Token::Dedent, "DEDENT (fim das cláusulas lambda)")?;
        Ok(clauses)
    }

    /// Parse uma cláusula lambda: `lambda <patterns>: <body>`.
    /// Fase 5: body é expressão única (sem guards, sem with).
    /// Fase 6: body pode ser bloco indentado com guards + with.
    fn parse_lambda_clause(&mut self) -> Result<LambdaClause, FrontendError> {
        self.expect(&Token::Lambda, "`lambda`")?;

        // Parse patterns (1 ou mais, separados por espaço)
        let patterns = self.parse_patterns()?;

        // Expect `:`
        self.expect(&Token::Colon, "`:` após patterns da cláusula")?;

        // Fase 5: body é expressão única (sem guards).
        // Fase 6: se há INDENT após `:`, é bloco com guards + with.
        let body = if matches!(self.peek(), Token::Indent) {
            // Fase 6 implementará guards + with.
            // Por agora, todo!() — não deveria ser alcançado em Fase 5.
            todo!("Fase 6: parse_lambda_clause com guards + with")
        } else {
            parse_expr(self)?
        };

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

    fn parse_data_decl(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        self.expect(&Token::Data, "`data`")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("type name after `data`")),
        };

        // Parse fields: (field::Type field::Type) or ()
        let fields = self.parse_field_decls()?;

        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(Item::DataDecl {
            name,
            fields,
            directives,
        })
    }

    fn parse_field_decls(&mut self) -> Result<Vec<FieldDecl>, FrontendError> {
        let mut fields = Vec::new();
        self.expect(&Token::LParen, "`(`")?;

        if matches!(self.peek(), Token::RParen) {
            self.advance();
            return Ok(fields);
        }

        // Fields are: name::Type name::Type ...
        // (space-separated, not comma-separated per PRD syntax)
        loop {
            let name = match self.peek() {
                Token::Ident(s) => {
                    let n = s.clone();
                    self.advance();
                    n
                }
                _ => return Err(self.error("field name")),
            };
            self.expect(&Token::DoubleColon, "`::` for field type")?;
            let ty = self.parse_type_expr()?;
            fields.push(FieldDecl { name, ty });

            if matches!(self.peek(), Token::RParen) {
                self.advance();
                break;
            }
        }

        Ok(fields)
    }

    fn parse_enum_decl(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        self.expect(&Token::Enum, "`enum`")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("enum name after `enum`")),
        };

        // Variants are in an INDENT..DEDENT block
        let mut variants = Vec::new();

        // Expect INDENT
        self.expect(&Token::Indent, "INDENT (indented variants)")?;

        loop {
            // Skip StmtSep between variants
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }

            let variant_name = match self.peek() {
                Token::Ident(s) => {
                    let n = s.clone();
                    self.advance();
                    n
                }
                _ => return Err(self.error("variant name")),
            };

            // For Fio 1, variants are unitary (no payload)
            // Fio 4 will add payload types with `(Type)`
            variants.push(VariantDecl {
                name: variant_name,
                payload: None,
            });
        }

        self.expect(&Token::Dedent, "DEDENT (end of enum variants)")?;

        Ok(Item::EnumDecl {
            name,
            variants,
            directives,
        })
    }
}
