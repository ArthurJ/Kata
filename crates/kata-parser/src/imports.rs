//! Import e Export — parse_import_decl, parse_export_decl.
//!
//! Sintaxe:
//! ```text
//! import modulo.submodulo                  # módulo inteiro
//! import modulo.submodulo as alias         # com alias de módulo
//! import modulo.(Item1 Item2)              # seletivo
//! import modulo.(item1 as alias1 item2)    # seletivo com alias por item
//! import modulo.item as alias              # açúcar para modulo.(item as alias)
//!
//! export item1 item2 ...                   # export direto
//! export MOD.(itens)                       # reexportação
//! ```

use kata_ast::{ExportItem, ImportItem, Item, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    /// `import modulo.submodulo [as alias]` ou `import modulo.(items)`
    /// ou `import modulo.(item1 as alias1 item2)` ou `import modulo.item as alias`.
    pub(crate) fn parse_import_decl(&mut self) -> Result<Item, FrontendError> {
        self.expect(&Token::Import, "`import`")?;

        // Path: Ident (. Ident)*
        let mut path = Vec::new();
        let first = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("module name after `import`")),
        };
        path.push(first);

        while matches!(self.peek(), Token::Dot) {
            self.advance(); // consume .
            match self.peek() {
                Token::Ident(s) => {
                    path.push(s.clone());
                    self.advance();
                }
                Token::LParen => {
                    // Import seletivo: `import MOD.(Item1 Item2)` ou
                    // `import MOD.(item1 as alias1 item2)`.
                    self.advance(); // consume (
                    let mut items = Vec::new();
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    if !matches!(self.peek(), Token::RParen) {
                        loop {
                            let iname = match self.peek() {
                                Token::Ident(s) => {
                                    let n = s.clone();
                                    self.advance();
                                    n
                                }
                                _ => return Err(self.error("item name in import list")),
                            };

                            // Alias opcional: `item as alias`
                            let alias = if matches!(self.peek(), Token::As) {
                                self.advance();
                                match self.peek() {
                                    Token::Ident(s) => {
                                        let a = s.clone();
                                        self.advance();
                                        Some(a)
                                    }
                                    _ => {
                                        return Err(self.error("alias name after `as`"))
                                    }
                                }
                            } else {
                                None
                            };

                            items.push(ImportItem { name: iname, alias });

                            // Itens separados por espaço ou vírgula
                            if matches!(self.peek(), Token::Comma) {
                                self.advance();
                                while matches!(self.peek(), Token::StmtSep) {
                                    self.advance();
                                }
                                continue;
                            }
                            // Skip space (StmtSep) between items
                            while matches!(self.peek(), Token::StmtSep) {
                                self.advance();
                            }
                            if matches!(self.peek(), Token::RParen) {
                                break;
                            }
                        }
                    }
                    self.expect(&Token::RParen, "`)` após import items")?;

                    if matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }

                    return Ok(Item::ImportDecl {
                        path,
                        alias: None,
                        items: Some(items),
                    });
                }
                _ => return Err(self.error("module name or `(` after `.` in import")),
            }
        }

        // `as alias` opcional — pode ser alias do módulo inteiro
        // (quando path tem 1 componente) ou açúcar para import seletivo
        // de um item (quando path tem 2+ componentes).
        let alias = if matches!(self.peek(), Token::As) {
            self.advance();
            match self.peek() {
                Token::Ident(s) => {
                    let n = s.clone();
                    self.advance();
                    Some(n)
                }
                _ => return Err(self.error("alias name after `as`")),
            }
        } else {
            None
        };

        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        // Desambiguação: `import mod.item as alias`
        // - Se path tem 2+ componentes e alias é Some, é açúcar para
        //   `import mod.(item as alias)` — import seletivo de um item.
        // - Se path tem 1 componente e alias é Some, é alias do módulo.
        if path.len() >= 2 && alias.is_some() {
            // Açúcar: `import mod.item as alias` → `import mod.(item as alias)`
            let item_name = path.pop().unwrap();
            let alias_name = alias.unwrap();
            return Ok(Item::ImportDecl {
                path,
                alias: None,
                items: Some(vec![ImportItem {
                    name: item_name,
                    alias: Some(alias_name),
                }]),
            });
        }

        Ok(Item::ImportDecl {
            path,
            alias,
            items: None,
        })
    }

    /// `export item1 item2 ...` ou `export MOD.(itens)`.
    pub(crate) fn parse_export_decl(&mut self) -> Result<Item, FrontendError> {
        self.expect(&Token::Export, "`export`")?;

        // `export MOD.(itens)` — reexportação
        if matches!(self.peek(), Token::Ident(_)) && {
            // Lookahead: Ident . ( ... )
            let saved = self.pos;
            self.advance();
            let is_reexport = matches!(self.peek(), Token::Dot);
            self.pos = saved;
            is_reexport
        } {
            let mut path = Vec::new();
            match self.peek() {
                Token::Ident(s) => {
                    path.push(s.clone());
                    self.advance();
                }
                _ => return Err(self.error("module name after `export`")),
            }
            self.expect(&Token::Dot, "`.` after module name in reexport")?;
            self.expect(&Token::LParen, "`(` after `.` in reexport")?;

            let mut items = Vec::new();
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if !matches!(self.peek(), Token::RParen) {
                loop {
                    let iname = match self.peek() {
                        Token::Ident(s) => {
                            let n = s.clone();
                            self.advance();
                            n
                        }
                        _ => return Err(self.error("item name in export list")),
                    };
                    items.push(iname);
                    if matches!(self.peek(), Token::Comma) {
                        self.advance();
                        while matches!(self.peek(), Token::StmtSep) {
                            self.advance();
                        }
                        continue;
                    }
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    if matches!(self.peek(), Token::RParen) {
                        break;
                    }
                }
            }
            self.expect(&Token::RParen, "`)` após export items")?;
            if matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }

            let mod_name = path.pop().unwrap();
            return Ok(Item::ExportDecl {
                items: vec![ExportItem {
                    name: format!("{mod_name}.*"),
                    reexport_from: Some(mod_name),
                    reexport_items: Some(items),
                }],
            });
        }

        // `export item1 item2 ...` — export direto
        let mut items = Vec::new();
        while matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }
        if !matches!(self.peek(), Token::StmtSep | Token::Eof) {
            loop {
                let iname = match self.peek() {
                    Token::Ident(s) => {
                        let n = s.clone();
                        self.advance();
                        n
                    }
                    _ => return Err(self.error("item name in export list")),
                };
                items.push(ExportItem {
                    name: iname,
                    reexport_from: None,
                    reexport_items: None,
                });
                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    continue;
                }
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                if matches!(self.peek(), Token::StmtSep | Token::Eof) {
                    break;
                }
            }
        }

        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(Item::ExportDecl { items })
    }
}