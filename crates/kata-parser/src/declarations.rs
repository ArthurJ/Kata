//! Declarations — parse_module, directives, sig, data, enum, fields.

use kata_ast::{Item, Module, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

/// Cria `ModuleEntry` a partir de `Spanned<Item>`, anexando `diag_pragmas`
/// (pragmas posicionais de diagnóstico) ao entry. Se vazio, usa `ModuleEntry::new`.
fn entry_from(spanned: Spanned<Item>, diag: Vec<kata_ast::Pragma>) -> kata_ast::ModuleEntry {
    if diag.is_empty() {
        kata_ast::ModuleEntry::new(spanned)
    } else {
        kata_ast::ModuleEntry::with_pragmas(spanned, diag)
    }
}

impl Parser {
    pub(crate) fn parse_module(&mut self) -> Result<Module, FrontendError> {
        let mut items: Vec<kata_ast::ModuleEntry> = Vec::new();
        let mut all_pragmas: Vec<kata_ast::Pragma> = Vec::new();

        while !self.at_eof() {
            // Skip leading statement separators
            if matches!(self.peek(), Token::StmtSep) {
                self.advance();
                continue;
            }

            // Collect pragmas (zero or more #!token ... prefixes)
            let pragmas = self.parse_pragmas()?;

            // Separar pragmas posicionais: TestSpec (anexa à action seguinte
            // via campo dedicado) e DiagnosticControl (anexa ao ModuleEntry
            // de qualquer item seguinte) dos demais (vão para Module.pragmas,
            // escopo module-global).
            let (test_pragmas, remaining): (Vec<_>, Vec<_>) = pragmas
                .into_iter()
                .partition(|p| matches!(p, kata_ast::Pragma::TestSpec(_)));
            let (diag_pragmas, other_pragmas): (Vec<_>, Vec<_>) = remaining
                .into_iter()
                .partition(|p| matches!(p, kata_ast::Pragma::DiagnosticControl(_)));
            all_pragmas.extend(other_pragmas);

            // Skip statement separators that may appear after pragmas
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
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
                // Pragmas órfãos (sem item seguinte). TestSpec sem action
                // é erro; DiagnosticControl sem item seguinte vai para
                // Module.pragmas (escopo module-global).
                all_pragmas.extend(test_pragmas);
                all_pragmas.extend(diag_pragmas);
                break;
            }

            let item_start = self.peek_span();

            match self.peek() {
                Token::Data => {
                    let item = self.parse_data_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Enum => {
                    let item = self.parse_enum_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Alias => {
                    let item = self.parse_alias_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Action => {
                    let item = self.parse_action_decl(directives, test_pragmas)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Directive => {
                    let item = self.parse_directive_decl()?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Interface => {
                    let item = self.parse_interface_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
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
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Export => {
                    let item = self.parse_export_decl()?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Constant => {
                    let item = self.parse_constant_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Let => {
                    if self.repl_mode {
                        // REPL mode: `let` no top level é aceito como EntryExpr
                        // (PRD §2.5 — o REPL não é top-level de módulo).
                        // `@comptime` foi removido — se presente, erro.
                        if directives.iter().any(|d| d.name == "comptime") {
                            return Err(self.error(
                                "`@comptime` foi removido. Use `constant` para constantes de módulo, ou remova `@comptime`.",
                            ));
                        }
                        let expr = self.parse_let()?;
                        items.push(entry_from(
                            Spanned::new(Item::EntryExpr(expr.clone()), expr.span),
                            diag_pragmas,
                        ));
                    } else {
                        // `let` no top level é proibido — usar `constant`.
                        return Err(self.error(
                            "`let` não é permitido no top level. Use `constant` para constantes de módulo, ou mova o código para uma action.",
                        ));
                    }
                }
                _ => {
                    // Could be a signature (name :: Type...), an implements
                    // decl (Tipo implements IFACE), a refines decl
                    // (TipoRefinado refines IFACE), or an expression.
                    if self.is_implements_start() {
                        let item = self.parse_implements_decl(directives)?;
                        items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                    } else if self.is_refines_start() {
                        let item = self.parse_refines_decl(directives)?;
                        items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                    } else if self.is_signature_start() {
                        let item = self.parse_sig(directives)?;
                        items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                    } else {
                        // Entry expression
                        let expr = parse_expr(self)?;
                        // Consume trailing separators
                        while matches!(self.peek(), Token::StmtSep) {
                            self.advance();
                        }
                        // `@comptime` foi removido (PRD-constant ).
                        if directives.iter().any(|d| d.name == "comptime") {
                            return Err(self.error(
                                "`@comptime` foi removido. Use `constant` para constantes de módulo, ou remova `@comptime` — o fold automático otimiza chamadas puras com args literais.",
                            ));
                        }
                        items.push(entry_from(
                            Spanned::new(Item::EntryExpr(expr.clone()), expr.span),
                            diag_pragmas,
                        ));
                    }
                }
            }
        }

        Ok(Module {
            items,
            pragmas: all_pragmas,
        })
    }

    /// Parse apenas declarações — skipa entry exprs e top-level lets.
    ///
    /// Usado pelo Pass 1 do ciclo de dois passes. Reconhece
    /// declarações pelos mesmos tokens iniciais que `parse_module`:
    /// `data`, `enum`, `alias`, `action`, `interface`, `implements`,
    /// `refines`, `sig` (Ident ::), `import`, `export`. Tudo else é
    /// skipado até o próximo `StmtSep` ou `Eof`.
    pub(crate) fn parse_module_decls_only(&mut self) -> Result<Module, FrontendError> {
        let mut items: Vec<kata_ast::ModuleEntry> = Vec::new();
        let mut all_pragmas: Vec<kata_ast::Pragma> = Vec::new();

        while !self.at_eof() {
            // Skip leading statement separators
            if matches!(self.peek(), Token::StmtSep) {
                self.advance();
                continue;
            }

            // Collect pragmas (zero or more #!token ... prefixes)
            let pragmas = self.parse_pragmas()?;

            // Separar TestSpec (anexa à action) dos demais. DiagnosticControl
            // anexa ao ModuleEntry de qualquer item seguinte.
            let (test_pragmas, remaining): (Vec<_>, Vec<_>) = pragmas
                .into_iter()
                .partition(|p| matches!(p, kata_ast::Pragma::TestSpec(_)));
            let (diag_pragmas, other_pragmas): (Vec<_>, Vec<_>) = remaining
                .into_iter()
                .partition(|p| matches!(p, kata_ast::Pragma::DiagnosticControl(_)));
            all_pragmas.extend(other_pragmas);

            // Skip statement separators that may appear after pragmas
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }

            // Collect directives (zero or more @name ... prefixes)
            let directives = self.parse_directives()?;

            // Skip statement separators that may appear after directives
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }

            if matches!(self.peek(), Token::Eof) {
                all_pragmas.extend(test_pragmas);
                all_pragmas.extend(diag_pragmas);
                break;
            }

            let item_start = self.peek_span();

            match self.peek() {
                Token::Data => {
                    let item = self.parse_data_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Enum => {
                    let item = self.parse_enum_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Alias => {
                    let item = self.parse_alias_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Action => {
                    let item = self.parse_action_decl(directives, test_pragmas)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Directive => {
                    let item = self.parse_directive_decl()?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Interface => {
                    let item = self.parse_interface_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Import => {
                    let item = self.parse_import_decl()?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Export => {
                    let item = self.parse_export_decl()?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Constant => {
                    let item = self.parse_constant_decl(directives)?;
                    items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                }
                Token::Implements => {
                    return Err(self.error("expected type name before `implements`"));
                }
                _ => {
                    if self.is_implements_start() {
                        let item = self.parse_implements_decl(directives)?;
                        items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                    } else if self.is_refines_start() {
                        let item = self.parse_refines_decl(directives)?;
                        items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                    } else if self.is_signature_start() {
                        let item = self.parse_sig(directives)?;
                        items.push(entry_from(Spanned::new(item, item_start), diag_pragmas));
                    } else {
                        // Entry expr ou top-level let — skipar tokens
                        // até o próximo StmtSep ou Eof. diag_pragmas é órfão
                        // (não há item para anexar) — vai para Module.pragmas.
                        all_pragmas.extend(diag_pragmas);
                        while !self.at_eof() && !matches!(self.peek(), Token::StmtSep) {
                            self.advance();
                        }
                    }
                }
            }
        }

        Ok(Module {
            items,
            pragmas: all_pragmas,
        })
    }

    /// Parse module com error recovery de top-level items.
    ///
    /// Igual ao `parse_module`, mas quando um item falha, registra o erro
    /// em `errors` e skipa tokens até o próximo `StmtSep` ou `Eof`, então
    /// continua. Retorna `Ok(Module)` com os items parseados com sucesso
    /// (pode ser vazio se tudo falhou) e `Vec<FrontendError>` com os erros.
    pub(crate) fn parse_module_with_recovery(&mut self) -> (Module, Vec<FrontendError>) {
        let mut items: Vec<kata_ast::ModuleEntry> = Vec::new();
        let mut errors: Vec<FrontendError> = Vec::new();
        let mut all_pragmas: Vec<kata_ast::Pragma> = Vec::new();

        while !self.at_eof() {
            // Skip leading statement separators
            if matches!(self.peek(), Token::StmtSep) {
                self.advance();
                continue;
            }

            // Collect pragmas (zero or more #!token ... prefixes)
            let pragmas = match self.parse_pragmas() {
                Ok(p) => p,
                Err(e) => {
                    errors.push(e);
                    self.sync_to_stmt_sep();
                    continue;
                }
            };
            // Separar TestSpec (anexa à action) dos demais. DiagnosticControl
            // anexa ao ModuleEntry de qualquer item seguinte.
            let (test_pragmas, remaining): (Vec<_>, Vec<_>) = pragmas
                .into_iter()
                .partition(|p| matches!(p, kata_ast::Pragma::TestSpec(_)));
            let (diag_pragmas, other_pragmas): (Vec<_>, Vec<_>) = remaining
                .into_iter()
                .partition(|p| matches!(p, kata_ast::Pragma::DiagnosticControl(_)));
            all_pragmas.extend(other_pragmas);

            // Skip statement separators that may appear after pragmas
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
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
                all_pragmas.extend(test_pragmas);
                all_pragmas.extend(diag_pragmas);
                break;
            }

            let item_start = self.peek_span();

            match self.peek() {
                Token::Data => match self.parse_data_decl(directives) {
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Enum => match self.parse_enum_decl(directives) {
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },

                Token::Alias => match self.parse_alias_decl(directives) {
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Action => match self.parse_action_decl(directives, test_pragmas) {
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Directive => match self.parse_directive_decl() {
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Interface => match self.parse_interface_decl(directives) {
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
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
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Export => match self.parse_export_decl() {
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Constant => match self.parse_constant_decl(directives) {
                    Ok(item) => items.push(entry_from(Spanned::new(item, item_start), diag_pragmas)),
                    Err(e) => {
                        errors.push(e);
                        self.sync_to_stmt_sep();
                    }
                },
                Token::Let => {
                    errors.push(self.error(
                        "`let` não é permitido no top level. Use `constant` para constantes de módulo, ou mova o código para uma action.",
                    ));
                    self.sync_to_stmt_sep();
                }
                _ => {
                    // Signature, implements, refines, ou expression
                    if self.is_implements_start() {
                        match self.parse_implements_decl(directives) {
                            Ok(item) => {
                                items.push(entry_from(Spanned::new(item, item_start), diag_pragmas))
                            }
                            Err(e) => {
                                errors.push(e);
                                self.sync_to_stmt_sep();
                            }
                        }
                    } else if self.is_refines_start() {
                        match self.parse_refines_decl(directives) {
                            Ok(item) => {
                                items.push(entry_from(Spanned::new(item, item_start), diag_pragmas))
                            }
                            Err(e) => {
                                errors.push(e);
                                self.sync_to_stmt_sep();
                            }
                        }
                    } else if self.is_signature_start() {
                        match self.parse_sig(directives) {
                            Ok(item) => {
                                items.push(entry_from(Spanned::new(item, item_start), diag_pragmas))
                            }
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
                                // `@comptime` foi removido (PRD-constant ).
                                if directives.iter().any(|d| d.name == "comptime") {
                                    errors.push(self.error(
                                        "`@comptime` foi removido. Use `constant` para constantes de módulo, ou remova `@comptime`.",
                                    ));
                                    self.sync_to_stmt_sep();
                                    continue;
                                }
                                items.push(entry_from(
                                    Spanned::new(Item::EntryExpr(expr.clone()), expr.span),
                                    diag_pragmas,
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

        (
            Module {
                items,
                pragmas: all_pragmas,
            },
            errors,
        )
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
}
