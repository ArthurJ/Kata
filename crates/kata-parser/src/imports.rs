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
    ///
    /// Aceita prefixos especiais:
    /// - `import super.X` — sobe um nível na árvore de módulos
    /// - `import super.super.X` — sobe dois níveis
    /// - `import stdlib.X` — stdlib built-in explícita
    ///
    /// `super` é keyword (`Token::Super`). `stdlib` é `Token::Ident("stdlib")`
    /// com handling especial em resolution. Ambos são representados no
    /// `path: Vec<String>` como strings `"super"` e `"stdlib"`.
    pub(crate) fn parse_import_decl(&mut self) -> Result<Item, FrontendError> {
        self.expect(&Token::Import, "`import`")?;

        // Prefixo especial: super* ou stdlib
        let mut path = Vec::new();
        let mut has_super = false;
        let mut has_stdlib = false;

        match self.peek() {
            Token::Super => {
                // Consumir todos os `super` prefixos
                loop {
                    path.push("super".to_string());
                    self.advance();
                    has_super = true;
                    if !matches!(self.peek(), Token::Dot) {
                        return Err(self.error("module name after `super.`"));
                    }
                    self.advance(); // consume .
                    if matches!(self.peek(), Token::Super) {
                        continue;
                    }
                    break;
                }
            }
            Token::Ident(s) if s == "stdlib" => {
                path.push("stdlib".to_string());
                self.advance();
                has_stdlib = true;
                if !matches!(self.peek(), Token::Dot) {
                    return Err(self.error("module name after `stdlib.`"));
                }
                self.advance(); // consume .
            }
            _ => {}
        }

        // Path normal: Ident (. Ident)*
        // Se já consumimos prefixo (super/stdlib), o próximo deve ser Ident
        let first = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("module name after `import`")),
        };
        path.push(first);

        // Validação: super e stdlib não coexistem
        if has_super && has_stdlib {
            return Err(self.error("`super` and `stdlib` cannot coexist in import path"));
        }

        while matches!(self.peek(), Token::Dot) {
            self.advance(); // consume .
            match self.peek() {
                Token::Ident(s) => {
                    path.push(s.clone());
                    self.advance();
                }
                Token::Super => {
                    // `super` após componente normal é erro:
                    // `import math.super` não faz sentido
                    return Err(
                        self.error("`super` can only appear at the start of an import path")
                    );
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
                                    _ => return Err(self.error("alias name after `as`")),
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
        // Nota: path com prefixo super/stdlib tem 2+ componentes sempre
        // (prefixo + pelo menos 1 normal), então `import super.mod as alias`
        // é açúcar para `import super.(mod as alias)` — seletivo de um item.
        // Mas isso não faz sentido: `super` não é um módulo. O açúcar só
        // aplica se os componentes não-prefixo são 2+. Para super/stdlib,
        // alias é sempre alias do módulo inteiro.
        let non_prefix_len = if has_super {
            // super count: todos os "super" no início
            path.iter().take_while(|s| s == &"super").count()
        } else if has_stdlib {
            1 // "stdlib"
        } else {
            0
        };
        let normal_components = path.len() - non_prefix_len;

        if normal_components >= 2
            && let Some(alias_name) = alias
        {
            // Açúcar: `import mod.item as alias` → `import mod.(item as alias)`
            let item_name = path
                .pop()
                .expect("path tem >=2 componentes normais, pop é infalível");
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
