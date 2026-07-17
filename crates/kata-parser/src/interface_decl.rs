//! Interface declarations — `interface` and `implements` parsing.

use kata_ast::{Directive, ImplMethod, InterfaceSig, Item, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    /// `interface NOME implements SUPER1 SUPER2 ...` + bloco indentado
    /// de assinaturas. Sintaxe:
    /// ```text
    /// interface NUM implements ORD EQ
    ///     + :: NUM NUM => NUM
    ///     - :: NUM NUM => NUM
    /// ```
    /// Ou com type params:
    /// ```text
    /// interface ITERABLE::A
    ///     next :: Self => Optional::A
    /// ```
    pub fn parse_interface_decl(
        &mut self,
        _directives: Vec<Directive>,
    ) -> Result<Item, FrontendError> {
        self.expect(&Token::Interface, "`interface`")?;

        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("interface name after `interface`")),
        };

        // Type params opcionais: `ITERABLE::(A)` (tupla) ou `ITERABLE::A` (single).
        // `::` marca a fronteira; `()` delimita tuplas, nada mais.
        let type_params = if matches!(self.peek(), Token::DoubleColon) {
            self.advance(); // consume ::
            if matches!(self.peek(), Token::LParen) {
                self.advance(); // consume (
                let mut params = Vec::new();
                // Skip newlines after (
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                if !matches!(self.peek(), Token::RParen) {
                    loop {
                        let pname = match self.peek() {
                            Token::Ident(s) => {
                                let n = s.clone();
                                self.advance();
                                n
                            }
                            _ => return Err(self.error("type param name")),
                        };
                        params.push(pname);
                        if matches!(self.peek(), Token::Comma) {
                            self.advance();
                            while matches!(self.peek(), Token::StmtSep) {
                                self.advance();
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(&Token::RParen, "`)` após type params")?;
                params
            } else {
                // `ITERABLE::A` — single param sem parênteses.
                let pname = match self.peek() {
                    Token::Ident(s) => {
                        let n = s.clone();
                        self.advance();
                        n
                    }
                    _ => return Err(self.error("type param name after `::`")),
                };
                vec![pname]
            }
        } else {
            Vec::new()
        };

        // Supertraits opcionais: `implements ORD EQ`
        let mut supertraits = Vec::new();
        if matches!(self.peek(), Token::Implements) {
            self.advance(); // consume implements
            // Supertraits são Idents até INDENT ou Eof
            while !matches!(self.peek(), Token::Indent | Token::Eof | Token::StmtSep) {
                match self.peek() {
                    Token::Ident(s) => {
                        supertraits.push(s.clone());
                        self.advance();
                    }
                    _ => break,
                }
            }
        }

        // Bloco indentado de assinaturas
        self.expect(&Token::Indent, "INDENT (interface signatures)")?;
        let mut signatures = Vec::new();
        loop {
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }

            // Assinatura: `name :: Type1 Type2 ... => RetType`
            let sig_name = match self.peek() {
                Token::Ident(s) => {
                    let n = s.clone();
                    self.advance();
                    n
                }
                _ => return Err(self.error("method name in interface")),
            };
            self.expect(&Token::DoubleColon, "`::` in interface signature")?;

            let mut params = Vec::new();
            while !matches!(self.peek(), Token::FatArrow | Token::Eof) {
                params.push(self.parse_type_expr()?);
            }
            self.expect(&Token::FatArrow, "`=>` in interface signature")?;
            let ret = self.parse_type_expr()?;

            signatures.push(InterfaceSig {
                name: sig_name,
                params,
                ret,
            });
        }
        self.expect(&Token::Dedent, "DEDENT (end of interface)")?;

        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(Item::InterfaceDecl {
            name,
            supertraits,
            type_params,
            signatures,
        })
    }

    /// `Tipo implements Interface` + bloco indentado com métodos.
    /// Sintaxe:
    /// ```text
    /// Complex implements NUM
    ///     + :: Complex Complex => Complex
    ///         lambda a b: Complex (+ a.re b.re) (+ a.im b.im)
    ///     - :: Complex Complex => Complex @ffi("kata_rt_complex_sub")
    /// ```
    /// Ou com type params:
    /// ```text
    /// List::A implements ITERABLE::A
    ///     next :: List::A => Optional::A
    ///         lambda lst: ...
    /// ```
    pub fn parse_implements_decl(
        &mut self,
        _directives: Vec<Directive>,
    ) -> Result<Item, FrontendError> {
        // Nome do tipo (primeiro token)
        let type_name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("type name before `implements`")),
        };

        // Type params do tipo opcionais: `List::(A)` (tupla) ou `List::A` (single).
        let type_params = if matches!(self.peek(), Token::DoubleColon) {
            self.advance(); // consume ::
            if matches!(self.peek(), Token::LParen) {
                self.advance(); // consume (
                let mut params = Vec::new();
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                if !matches!(self.peek(), Token::RParen) {
                    loop {
                        let pname = match self.peek() {
                            Token::Ident(s) => {
                                let n = s.clone();
                                self.advance();
                                n
                            }
                            _ => return Err(self.error("type param name")),
                        };
                        params.push(pname);
                        if matches!(self.peek(), Token::Comma) {
                            self.advance();
                            while matches!(self.peek(), Token::StmtSep) {
                                self.advance();
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(&Token::RParen, "`)` após type params")?;
                params
            } else {
                // `List::A` — single param sem parênteses.
                let pname = match self.peek() {
                    Token::Ident(s) => {
                        let n = s.clone();
                        self.advance();
                        n
                    }
                    _ => return Err(self.error("type param name after `::`")),
                };
                vec![pname]
            }
        } else {
            Vec::new()
        };

        // `implements`
        self.expect(&Token::Implements, "`implements`")?;

        // Nome da interface
        let interface_name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("interface name after `implements`")),
        };

        // Params da interface opcionais: `ITERABLE::(A)` (tupla) ou `ITERABLE::A` (single).
        let iface_params = if matches!(self.peek(), Token::DoubleColon) {
            self.advance(); // consume ::
            if matches!(self.peek(), Token::LParen) {
                self.advance(); // consume (
                let mut params = Vec::new();
                while matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                if !matches!(self.peek(), Token::RParen) {
                    loop {
                        let pname = match self.peek() {
                            Token::Ident(s) => {
                                let n = s.clone();
                                self.advance();
                                n
                            }
                            _ => return Err(self.error("iface param name")),
                        };
                        params.push(pname);
                        if matches!(self.peek(), Token::Comma) {
                            self.advance();
                            while matches!(self.peek(), Token::StmtSep) {
                                self.advance();
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(&Token::RParen, "`)` após iface params")?;
                params
            } else {
                // `ITERABLE::A` — single param sem parênteses.
                let pname = match self.peek() {
                    Token::Ident(s) => {
                        let n = s.clone();
                        self.advance();
                        n
                    }
                    _ => return Err(self.error("iface param name after `::`")),
                };
                vec![pname]
            }
        } else {
            Vec::new()
        };

        // Bloco indentado de métodos
        self.expect(&Token::Indent, "INDENT (implements methods)")?;
        let mut methods = Vec::new();
        loop {
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }

            // Assinatura: `name :: Type1 Type2 ... => RetType [@ffi(...)]`
            let method_name = match self.peek() {
                Token::Ident(s) => {
                    let n = s.clone();
                    self.advance();
                    n
                }
                _ => return Err(self.error("method name in implements")),
            };
            self.expect(&Token::DoubleColon, "`::` in method signature")?;

            let mut params = Vec::new();
            while !matches!(self.peek(), Token::FatArrow | Token::Eof) {
                params.push(self.parse_type_expr()?);
            }
            self.expect(&Token::FatArrow, "`=>` in method signature")?;
            let ret = self.parse_type_expr()?;

            // Diretivas opcionais APÓS o tipo de retorno: @ffi, @commutative
            let method_directives = self.parse_directives()?;

            // Corpo: lambda indentado (Some) ou apenas diretivas @ffi (None)
            let body = if matches!(self.peek(), Token::Indent) {
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

            if matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }

            methods.push(ImplMethod {
                name: method_name,
                params,
                ret,
                directives: method_directives,
                body,
            });
        }
        self.expect(&Token::Dedent, "DEDENT (end of implements)")?;

        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(Item::ImplementsDecl {
            type_name,
            type_params,
            interface_name,
            iface_params,
            methods,
        })
    }
}
