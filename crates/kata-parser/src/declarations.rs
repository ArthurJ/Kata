//! Declarations — parse_module, directives, sig, data, enum, fields.

use kata_ast::{
    Directive, Expr, FieldDecl, Item, LambdaClause, Module, RefinedDecl, Spanned, Token,
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
                Token::Alias => {
                    let item = self.parse_alias_decl(directives)?;
                    items.push(Spanned::new(item, item_start));
                }
                Token::Action => {
                    let item = self.parse_action_decl(directives)?;
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

    fn parse_data_decl(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        self.expect(&Token::Data, "`data`")?;

        // Fio 6: disambiguação via lookahead de 1 token.
        // `data Name (...)` → struct normal (Ident após data).
        // `data (Int, > _ 0) as PositiveInt` → refined (LParen após data).
        if matches!(self.peek(), Token::LParen) {
            return self.parse_refined_decl(directives);
        }

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
            refined: None,
        })
    }

    /// `data (Base, pred1, pred2, ...) as Name` — tipo refinado.
    /// O conteúdo de `()` é: TypeExpr (base) `,` Expr (`,` Expr)*.
    fn parse_refined_decl(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        self.expect(&Token::LParen, "`(` in refined declaration")?;

        // Primeiro elemento: TypeExpr (base)
        let base_ty = self.parse_type_expr()?;

        // Se já é `)`, refined sem predicados — não faz sentido, mas não crasha.
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            return Err(self.error("refined declaration precisa de pelo menos um predicado"));
        }

        // Restante: `,` Expr (`,` Expr)*
        let mut predicates = Vec::new();
        while matches!(self.peek(), Token::Comma) {
            self.advance(); // consume ,
            // Skip newlines after comma (multiline refined)
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::RParen) {
                break; // trailing comma
            }
            let pred = self.parse_expr_for_predicate()?;
            predicates.push(pred);
        }

        self.expect(&Token::RParen, "`)` in refined declaration")?;

        // `as Name`
        self.expect(&Token::As, "`as` in refined declaration")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("type name after `as`")),
        };

        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(Item::DataDecl {
            name,
            fields: Vec::new(),
            directives,
            refined: Some(RefinedDecl {
                base_ty,
                predicates,
            }),
        })
    }

    /// Parse uma expressão predicado: `> _ 0`, `<= _ 100`, etc.
    /// Usa `parse_expr` do módulo de expressões — os operadores `<`, `>`, `=`
    /// são `Token::Ident` em Kata, então o parser de expressões já os trata
    /// como callable em notação prefixa.
    fn parse_expr_for_predicate(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        parse_expr(self)
    }

    /// Disambiguação: o conteúdo de `()` após nome de variante é predicado
    /// ou payload? Predicado começa com operador de comparação ou `_` (Hole).
    /// Payload começa com TypeExpr (Ident CamelCase, `(`, etc).
    fn is_predicate_start(&self) -> bool {
        match self.peek() {
            Token::Ident(s) => matches!(s.as_str(), "<" | ">" | "<=" | ">=" | "=" | "_"),
            _ => false,
        }
    }

    /// `alias Target as NewName` — cria um newtype.
    fn parse_alias_decl(&mut self, _directives: Vec<Directive>) -> Result<Item, FrontendError> {
        self.expect(&Token::Alias, "`alias`")?;
        let target = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("target type name after `alias`")),
        };
        self.expect(&Token::As, "`as`")?;
        let new_name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("new name after `as`")),
        };
        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }
        Ok(Item::AliasDecl { target, new_name })
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

            // Fio 6: disambiguação payload vs predicado.
            // `Ok(Int)` → payload = Some(TypeExpr), predicate = None
            // `Magreza(< _ 18.5)` → payload = None, predicate = Some(Expr)
            // Disambiguação: após `(`, se primeiro token é operador de comparação
            // (`<`, `>`, `<=`, `>=`, `=`) ou `_` (Hole), é predicado. Senão é payload.
            let (payload, predicate) = if matches!(self.peek(), Token::LParen) {
                self.advance(); // consume (
                if self.is_predicate_start() {
                    let pred = parse_expr(self)?;
                    self.expect(&Token::RParen, "`)` após predicado")?;
                    (None, Some(pred))
                } else {
                    let ty = self.parse_type_expr()?;
                    self.expect(&Token::RParen, "`)` após tipo do payload")?;
                    (Some(ty), None)
                }
            } else {
                (None, None)
            };

            variants.push(VariantDecl {
                name: variant_name,
                payload,
                predicate,
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
