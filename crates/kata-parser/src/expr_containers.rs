//! Coleções e control flow — list/range/array literals, loop, for-in.
//!
//! Funções de parsing para estruturas de dados compostas e laços:
//! - `[1 2 3]` (ListLit), `[a..s..b]` / `[a..s..=b]` (RangeLit)
//! - `{1 2 3}` (ArrayLit)
//! - `loop` (laço infinito, exclusivo de Actions)
//! - `for x in colecao` (iteração, exclusivo de Actions)

use kata_ast::{Expr, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expr_apply::parse_expr;

impl Parser {
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
