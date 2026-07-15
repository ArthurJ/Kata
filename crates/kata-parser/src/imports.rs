//! Import e Export — parse_import_decl, parse_export_decl.
//!
//! Sintaxe:
//! ```text
//! import modulo.submodulo                  # módulo inteiro
//! import modulo.submodulo as alias         # com alias
//! import modulo.(Item1 Item2)              # seletivo
//!
//! export item1 item2 ...                   # export direto
//! export MOD.(itens)                       # reexportação
//! ```

use kata_ast::{ExportItem, Item, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    /// `import modulo.submodulo [as alias]` ou `import modulo.(items)`.
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
                    // Import seletivo: `import MOD.(Item1 Item2)`
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
                            items.push(iname);
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

        // `as alias` opcional
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

        Ok(Item::ImportDecl {
            path,
            alias,
            items: None,
        })
    }

    /// `export item1 item2 ...` ou `export MOD.(itens)`.
    pub(crate) fn parse_export_decl(&mut self) -> Result<Item, FrontendError> {
        self.expect(&Token::Export, "`export`")?;

        let mut items = Vec::new();

        // Export direto: `export item1 item2 ...`
        // Reexportação: `export MOD.(itens)`
        // Ambos começam com Ident.
        loop {
            let name = match self.peek() {
                Token::Ident(s) => {
                    let n = s.clone();
                    self.advance();
                    n
                }
                Token::StmtSep | Token::Eof => break,
                _ => return Err(self.error("item name after `export`")),
            };

            // Reexportação: `MOD.(itens)`
            if matches!(self.peek(), Token::Dot) {
                self.advance(); // consume .
                if matches!(self.peek(), Token::LParen) {
                    self.advance(); // consume (
                    let mut reexport_items = Vec::new();
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    if !matches!(self.peek(), Token::RParen) {
                        loop {
                            let rname = match self.peek() {
                                Token::Ident(s) => {
                                    let n = s.clone();
                                    self.advance();
                                    n
                                }
                                _ => return Err(self.error("item name in reexport list")),
                            };
                            reexport_items.push(rname);
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
                    self.expect(&Token::RParen, "`)` após reexport items")?;

                    items.push(ExportItem {
                        name: String::new(), // nome do MOD já consumido
                        reexport_from: Some(name),
                        reexport_items: Some(reexport_items),
                    });
                } else {
                    return Err(self.error("`(` after `.` in export"));
                }
            } else {
                items.push(ExportItem {
                    name,
                    reexport_from: None,
                    reexport_items: None,
                });
            }

            // Itens separados por espaço — StmtSep não é espaço aqui,
            // é quebra de linha. Mas em `export + - TipoX`, os itens
            // estão na mesma linha, separados por espaço.
            // O lexer não emite StmtSep entre itens na mesma linha.
            // Continua coletando até StmtSep ou Eof.
            if matches!(self.peek(), Token::StmtSep | Token::Eof) {
                break;
            }
        }

        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(Item::ExportDecl { items })
    }
}
