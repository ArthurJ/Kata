//! Parser recursive-descent, prefix-only (sem Pratt parsing).
//!
//! Consome `Vec<TokenWithSpan>` do lexer e produz `Module` (AST plana).
//! A notação prefixa elimina precedência de operadores — `+`, `soma`,
//! `fatorial` são todos identificadores tratados identicamente.
//!
//! Aplicação é greedy: `f a b c` vira um único `Apply { callee: f, args: [a, b, c] }`.

use kata_ast::{
    Directive, DirectiveArg, DirectiveValue, Expr, FieldDecl, Item, Module, Span, Spanned, Token,
    TokenWithSpan, TypeExpr, VariantDecl,
};
use kata_diagnostics::{FrontendError, MietteSpan};

// ────────────────────────────────────────────────────────────────
// Parser state
// ────────────────────────────────────────────────────────────────

pub struct Parser {
    tokens: Vec<TokenWithSpan>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<TokenWithSpan>) -> Self {
        Parser { tokens, pos: 0 }
    }

    // ── Token access helpers ──────────────────────────────────────

    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn peek_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    /// Advance and return the span of the consumed token.
    fn advance(&mut self) -> Span {
        let span = self.tokens[self.pos].span;
        self.pos += 1;
        span
    }

    fn error(&self, expected: &str) -> FrontendError {
        let found = self.peek().clone();
        FrontendError::UnexpectedToken {
            expected: expected.to_string(),
            found: found.to_string(),
            span: MietteSpan(self.peek_span()),
        }
    }

    /// Consume a token matching the given predicate, returning its span.
    fn expect(&mut self, expected: &Token, label: &str) -> Result<Span, FrontendError> {
        if self.peek() == expected {
            Ok(self.advance())
        } else {
            Err(self.error(label))
        }
    }

    // ── Top-level parse ───────────────────────────────────────────

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
                        let expr = self.parse_expr()?;
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

        // Optional body (lambda) — for Fio 1, signatures typically have no body
        let body = None;

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

    // ── Expressions ───────────────────────────────────────────────

    /// Determine if the current token can start an expression.
    fn can_start_expr(&self) -> bool {
        matches!(
            self.peek(),
            Token::IntLit(_)
                | Token::FloatLit(_)
                | Token::TextLit(_)
                | Token::Ident(_)
                | Token::LParen
                | Token::Let
        )
    }

    fn parse_let(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Let, "`let`")?;
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("binding name after `let`")),
        };
        self.expect(&Token::BindAssign, "`:=`")?;
        let value = self.parse_expr()?;
        // Cover span
        let span = start.cover(value.span);
        Ok(Spanned::new(
            Expr::Let {
                name,
                value: Box::new(value),
            },
            span,
        ))
    }

    /// Parse a single expression atom (no application).
    fn parse_expr_atom(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        match self.peek().clone() {
            Token::IntLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::IntLit { text: s }, start))
            }
            Token::FloatLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::FloatLit { text: s }, start))
            }
            Token::TextLit(s) => {
                self.advance();
                Ok(Spanned::new(Expr::TextLit { text: s }, start))
            }
            Token::Let => self.parse_let(),
            Token::LParen => self.parse_paren_expr(),
            Token::Ident(name) => {
                self.advance();
                // Check for VariantQual: Ident :: Ident
                // But `::` is also TypeAscription (expr::Type).
                // Disambiguation: if the Ident after `::` is a known type name vs variant...
                // The parser can't know. The PRD says VariantQual is `Enum::Variant`.
                // TypeAscription is `expr::Type`.
                // Heuristic: `Ident::Ident` where both are capitalized → VariantQual
                // Actually, the PRD says "The parser doesn't know if Boolean is type or
                // module — produces VariantQual and typeck resolves."
                // But `3.14::Rational` is TypeAscription(FloatLit, Named(Rational)).
                // So: if left side is a literal → TypeAscription
                // If left side is an Ident and we see `::` followed by Ident → VariantQual
                // If left side is an Ident and we see `::` followed by something that's a
                // type expression (not a simple Ident that could be a variant) → ambiguous
                //
                // For Fio 1: the only use of `::` with an Ident on the left is VariantQual.
                // TypeAscription with `::` is only for literals (`3.14::Rational`).
                // So: Ident :: Ident → VariantQual
                //     Literal :: Type → TypeAscription
                if matches!(self.peek(), Token::DoubleColon) {
                    // Ident :: Ident → VariantQual
                    if let Some(next) = self.tokens.get(self.pos + 1) {
                        if let Token::Ident(variant) = &next.token {
                            let variant = variant.clone();
                            self.advance(); // consume ::
                            self.advance(); // consume variant Ident
                            let span = start.cover(self.tokens[self.pos - 1].span);
                            return Ok(Spanned::new(
                                Expr::VariantQual {
                                    enum_name: name,
                                    variant,
                                },
                                span,
                            ));
                        }
                    }
                }
                Ok(Spanned::new(Expr::Ident { name }, start))
            }
            _ => Err(self.error("expression")),
        }
    }

    /// Parse parenthesized expression: `()`, `(expr)`, `(a, b, c)`, `(a,)`
    fn parse_paren_expr(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::LParen, "`(`")?;

        // `()` = Unit
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            return Ok(Spanned::new(Expr::Unit, start));
        }

        // Parse first expression
        let first = self.parse_expr()?;

        // No comma → grouping
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            let span = start.cover(first.span);
            return Ok(Spanned::new(
                Expr::Grouping {
                    inner: Box::new(first),
                },
                span,
            ));
        }

        // Comma present → tuple
        let mut elements = vec![first];
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            // Trailing comma: `(a,)` → tuple of 1
            if matches!(self.peek(), Token::RParen) {
                break;
            }
            elements.push(self.parse_expr()?);
        }

        self.expect(&Token::RParen, "`)`")?;
        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(Expr::Tuple { elements }, span))
    }

    /// Parse an expression with greedy application.
    fn parse_expr(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        // Parse the callee/first expression
        let callee = self.parse_expr_post_ascription()?;

        // Greedily collect arguments
        let mut args = Vec::new();
        while self.can_start_expr() {
            args.push(self.parse_expr_atom_or_ascription()?);
        }

        if args.is_empty() {
            Ok(callee)
        } else {
            let span = callee.span.cover(args.last().unwrap().span);
            Ok(Spanned::new(
                Expr::Apply {
                    callee: Box::new(callee),
                    args,
                },
                span,
            ))
        }
    }

    /// Parse an atom, then check for `::Type` ascription.
    fn parse_expr_post_ascription(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let atom = self.parse_expr_atom()?;

        // Check for TypeAscription: expr::Type
        if matches!(self.peek(), Token::DoubleColon) {
            self.advance(); // consume ::
            let ty = self.parse_type_expr()?;
            let span = atom.span.cover(ty.span);
            return Ok(Spanned::new(
                Expr::TypeAscription {
                    expr: Box::new(atom),
                    ty,
                },
                span,
            ));
        }

        Ok(atom)
    }

    /// Parse an atom or ascription — used for arguments in Apply.
    /// Arguments don't greedily consume more arguments themselves.
    fn parse_expr_atom_or_ascription(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        self.parse_expr_post_ascription()
    }

    // ── Type expressions ──────────────────────────────────────────

    fn parse_type_expr(&mut self) -> Result<Spanned<TypeExpr>, FrontendError> {
        let start = self.peek_span();
        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance();
                Ok(Spanned::new(TypeExpr::Named(name), start))
            }
            Token::LParen => {
                self.advance(); // consume (
                // `()` = Unit type
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    return Ok(Spanned::new(TypeExpr::Unit, start));
                }
                let first = self.parse_type_expr()?;

                // No comma → grouping
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    let span = start.cover(first.span);
                    return Ok(Spanned::new(TypeExpr::Grouping(Box::new(first)), span));
                }

                // Comma present → could be Func type if ThinArrow follows
                let mut params = vec![first];
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    if matches!(self.peek(), Token::RParen) {
                        break;
                    }
                    params.push(self.parse_type_expr()?);
                }
                self.expect(&Token::RParen, "`)`")?;

                // Check for `->` (function type)
                if matches!(self.peek(), Token::ThinArrow) {
                    self.advance();
                    let ret = self.parse_type_expr()?;
                    let span = start.cover(ret.span);
                    return Ok(Spanned::new(
                        TypeExpr::Func {
                            params,
                            ret: Box::new(ret),
                        },
                        span,
                    ));
                }

                // No `->` — this could be ParamApp but we don't have the type name here.
                // For Fio 1, this case shouldn't arise. Return as grouping of first param
                // as a fallback (this is a simplification).
                let span = start.cover(params.last().unwrap().span);
                Ok(Spanned::new(
                    TypeExpr::Grouping(Box::new(params.into_iter().next().unwrap())),
                    span,
                ))
            }
            _ => Err(self.error("type expression")),
        }
    }
}

// ────────────────────────────────────────────────────────────────
// Public API
// ────────────────────────────────────────────────────────────────

/// Parse a token stream into a `Module`.
///
/// This is the main entry point. It consumes the tokens produced by the lexer
/// and produces a `Module` (list of `Spanned<Item>`).
pub fn parse(tokens: Vec<TokenWithSpan>) -> Result<Module, FrontendError> {
    let mut parser = Parser::new(tokens);
    parser.parse_module()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kata_lexer::lex;

    fn parse_src(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        parse(tokens).unwrap()
    }

    fn first_item(m: &Module) -> &Item {
        &m.items.first().expect("at least one item").node
    }

    #[test]
    fn apply_plus_1_2() {
        let m = parse_src("+ 1 2");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::Apply { callee, args } => {
                    assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                    assert_eq!(args.len(), 2);
                    assert_eq!(args[0].node, Expr::IntLit { text: "1".into() });
                    assert_eq!(args[1].node, Expr::IntLit { text: "2".into() });
                }
                other => panic!("expected Apply, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn let_binding() {
        let m = parse_src("let x := 42");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::Let { name, value } => {
                    assert_eq!(name, "x");
                    assert_eq!(value.node, Expr::IntLit { text: "42".into() });
                }
                other => panic!("expected Let, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn type_ascription_rational() {
        let m = parse_src("3.14::Rational");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::TypeAscription { expr, ty } => {
                    assert_eq!(
                        expr.node,
                        Expr::FloatLit {
                            text: "3.14".into()
                        }
                    );
                    assert_eq!(ty.node, TypeExpr::Named("Rational".into()));
                }
                other => panic!("expected TypeAscription, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn tuple_three_elements() {
        let m = parse_src("(1, 2, 3)");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::Tuple { elements } => {
                    assert_eq!(elements.len(), 3);
                    assert_eq!(elements[0].node, Expr::IntLit { text: "1".into() });
                    assert_eq!(elements[1].node, Expr::IntLit { text: "2".into() });
                    assert_eq!(elements[2].node, Expr::IntLit { text: "3".into() });
                }
                other => panic!("expected Tuple, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn grouping_single() {
        let m = parse_src("(42)");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::Grouping { inner } => {
                    assert_eq!(inner.node, Expr::IntLit { text: "42".into() });
                }
                other => panic!("expected Grouping, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn unit_lit() {
        let m = parse_src("()");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => assert_eq!(e.node, Expr::Unit),
            other => panic!("expected EntryExpr(Unit), got {other:?}"),
        }
    }

    #[test]
    fn variant_qual() {
        let m = parse_src("Boolean::True");
        let item = first_item(&m);
        match item {
            Item::EntryExpr(e) => match &e.node {
                Expr::VariantQual { enum_name, variant } => {
                    assert_eq!(enum_name, "Boolean");
                    assert_eq!(variant, "True");
                }
                other => panic!("expected VariantQual, got {other:?}"),
            },
            other => panic!("expected EntryExpr, got {other:?}"),
        }
    }

    #[test]
    fn data_decl_empty() {
        let m = parse_src("data Int ()");
        let item = first_item(&m);
        match item {
            Item::DataDecl {
                name,
                fields,
                directives,
            } => {
                assert_eq!(name, "Int");
                assert!(fields.is_empty());
                assert!(directives.is_empty());
            }
            other => panic!("expected DataDecl, got {other:?}"),
        }
    }

    #[test]
    fn enum_decl_variants() {
        let m = parse_src("enum Boolean\n    True\n    False");
        let item = first_item(&m);
        match item {
            Item::EnumDecl {
                name,
                variants,
                directives,
            } => {
                assert_eq!(name, "Boolean");
                assert_eq!(variants.len(), 2);
                assert_eq!(variants[0].name, "True");
                assert_eq!(variants[1].name, "False");
                assert!(directives.is_empty());
            }
            other => panic!("expected EnumDecl, got {other:?}"),
        }
    }

    #[test]
    fn sig_simple() {
        let m = parse_src("+ :: Int Int => Int");
        let item = first_item(&m);
        match item {
            Item::Sig {
                name,
                params,
                ret,
                directives,
                body,
            } => {
                assert_eq!(name, "+");
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
                assert_eq!(params[1].node, TypeExpr::Named("Int".into()));
                assert_eq!(ret.node, TypeExpr::Named("Int".into()));
                assert!(directives.is_empty());
                assert!(body.is_none());
            }
            other => panic!("expected Sig, got {other:?}"),
        }
    }

    #[test]
    fn directive_ffi() {
        // A directive alone is not an item — the parser needs an item after it.
        // In the prelude, @ffi precedes a Sig. Let's test with a full item.
        let m = parse_src("@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int");
        let item = first_item(&m);
        match item {
            Item::Sig {
                name, directives, ..
            } => {
                assert_eq!(name, "+");
                assert_eq!(directives.len(), 1);
                assert_eq!(directives[0].name, "ffi");
                assert_eq!(
                    directives[0].args,
                    vec![DirectiveArg::Str("kata_rt_bi_add".into())]
                );
            }
            other => panic!("expected Sig with directive, got {other:?}"),
        }
    }

    #[test]
    fn directive_associative_int() {
        let m = parse_src("@associative(0)\n+ :: Int Int => Int");
        let item = first_item(&m);
        match item {
            Item::Sig { directives, .. } => {
                assert_eq!(directives.len(), 1);
                assert_eq!(directives[0].name, "associative");
                assert_eq!(directives[0].args, vec![DirectiveArg::Int(0)]);
            }
            other => panic!("expected Sig, got {other:?}"),
        }
    }
}
