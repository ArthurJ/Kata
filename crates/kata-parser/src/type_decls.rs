//! Type declarations — `data`, `enum`, `alias`, refined types.
//!
//! Extraído de `declarations.rs` (Zeladoria Passo 6 D2). Contém os parses
//! de declarações de tipos: structs (`data`), enums, aliases e refined
//! types. `parse_module` despacha para cá.

use kata_ast::{Directive, Expr, FieldDecl, Item, RefinedDecl, Spanned, VariantDecl};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    pub(crate) fn parse_data_decl(
        &mut self,
        directives: Vec<Directive>,
    ) -> Result<Item, FrontendError> {
        self.expect(&Token::Data, "`data`")?;

        // Disambiguação via lookahead de 1 token.
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
    pub(crate) fn is_predicate_start(&self) -> bool {
        match self.peek() {
            Token::Ident(s) => matches!(s.as_str(), "<" | ">" | "<=" | ">=" | "=" | "_"),
            _ => false,
        }
    }

    /// `alias Target as NewName` — cria um newtype.
    pub(crate) fn parse_alias_decl(
        &mut self,
        _directives: Vec<Directive>,
    ) -> Result<Item, FrontendError> {
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

    pub(crate) fn parse_field_decls(&mut self) -> Result<Vec<FieldDecl>, FrontendError> {
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

    pub(crate) fn parse_enum_decl(
        &mut self,
        directives: Vec<Directive>,
    ) -> Result<Item, FrontendError> {
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

            // Disambiguação payload vs predicado.
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

// Re-export Token variants used above as bare names. The `impl Parser` block
// references `Token::Data`, `Token::LParen`, etc., which are accessible via
// `kata_ast::Token` — but the original file imported `Token` from `kata_ast`.
// We bring it into scope here via the `use` below (already in the `use` block
// above indirectly). To keep this self-contained, we re-import Token.
use kata_ast::Token;
