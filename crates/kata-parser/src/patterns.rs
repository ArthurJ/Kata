//! Patterns — parse_pattern para match arms e cláusulas lambda.
//!
//! Patterns são reusados integralmente entre `match` e cláusulas lambda.
//! Disambiguação no parser: `_` em posição de pattern → `Wildcard`.
//! `True` em posição de pattern → `Ident("True")` (typeck resolve via
//! `EnumRegistry` para `Variant` se for variante de enum do scrutinee).

use kata_ast::{Expr, Pattern, Spanned, Token};
use kata_diagnostics::FrontendError;

use crate::Parser;

impl Parser {
    /// Parse um pattern único.
    ///
    /// Patterns suportados:
    /// - `_` → Wildcard
    /// - `42`, `"texto"`, `3.14` → Literal
    /// - `Ident` → Ident (pode ser variante desqualificada — typeck resolve)
    /// - `Enum::Variant` → Variant (qualificado)
    /// - `(p1, p2, ...)` → Tuple
    /// - `[h : t]` → Cons (stub )
    /// - `[]` → Cons Nil (stub )
    pub(crate) fn parse_pattern(&mut self) -> Result<Spanned<Pattern>, FrontendError> {
        self.parse_pattern_inner(false)
    }

    /// Parse um pattern de match arm — difere de `parse_pattern` por tratar
    /// `Ident` seguido de sub-pattern como variante desqualificada com payload.
    ///
    /// Em match arms, cada braço tem exatamente um pattern, então `Ok v` só
    /// pode significar "variante Ok com payload v" — não há ambiguidade com
    /// múltiplos argumentos (como em lambda clauses).
    ///
    /// `Ident` sozinho (sem sub-pattern) continua como `Pattern::Ident` —
    /// o typeck resolve variantes unitárias via EnumRegistry.
    pub(crate) fn parse_match_pattern(&mut self) -> Result<Spanned<Pattern>, FrontendError> {
        self.parse_pattern_inner(true)
    }

    fn parse_pattern_inner(
        &mut self,
        allow_unqualified_variant: bool,
    ) -> Result<Spanned<Pattern>, FrontendError> {
        let start = self.peek_span();
        match self.peek().clone() {
            // `_` → Wildcard
            Token::Ident(s) if s == "_" => {
                self.advance();
                Ok(Spanned::new(Pattern::Wildcard, start))
            }
            // `Enum::Variant` → Variant qualificado (possivelmente com sub-pattern)
            // `snake_case::Type` → TypedIdent (type annotation em lambda params)
            Token::Ident(name) => {
                self.advance();
                if matches!(self.peek(), Token::DoubleColon)
                    && let Some(next) = self.tokens.get(self.pos + 1)
                    && let Token::Ident(variant) = &next.token
                {
                    // Disambiguação por casing:
                    // - name é snake_case (primeiro char lowercase) → type annotation
                    //   (`x::Int`, `n::Float`). Parsear como TypedIdent.
                    // - name é PascalCase → Enum::Variant (ex: `Result::Ok`).
                    if name.chars().next().is_some_and(|c| c.is_lowercase()) {
                        // snake_case::PascalCase → TypedIdent (type annotation)
                        self.advance(); // consume ::
                        let ty = self.parse_type_expr()?;
                        let span = start.cover(ty.span);
                        return Ok(Spanned::new(Pattern::TypedIdent { name, ty }, span));
                    }

                    let variant = variant.clone();
                    self.advance(); // consume ::
                    self.advance(); // consume variant

                    // Verificar se há um sub-pattern após a variante.
                    // `Result::Ok v` → Variant com payload sub-pattern.
                    // `Result::Ok(v)` → Variant com payload sub-pattern (entre parênteses).
                    // `Boolean::True` (sem sub-pattern) → Variant sem payload.
                    //
                    // Sub-patterns herdam `allow_unqualified_variant` do pai,
                    // para que `Some(Some(True))` aninhe corretamente.
                    //
                    // `Result::Ok(v)` com parênteses é tratado pelo ramo
                    // `can_start_pattern()` acima: `(` inicia pattern →
                    // `parse_pattern_inner` → `parse_tuple_pattern` →
                    // desembrulha `(v)` sem vírgula. O `else if LParen`
                    // anterior era código morto (`can_start_pattern` captura
                    // `(` antes) e foi removido.
                    let payload = if self.can_start_pattern() {
                        // Sub-pattern: `Result::Ok v` ou `Result::Ok(v)`
                        Some(vec![self.parse_pattern_inner(allow_unqualified_variant)?])
                    } else {
                        None
                    };

                    let end_span = self
                        .tokens
                        .get(self.pos - 1)
                        .map(|t| t.span)
                        .unwrap_or(start);
                    let span = start.cover(end_span);
                    return Ok(Spanned::new(
                        Pattern::Variant {
                            enum_name: name,
                            variant,
                            payload,
                        },
                        span,
                    ));
                }
                // Ident sem `::` — pode ser binding, variante unitária, ou
                // variante desqualificada com payload.
                //
                // Em match arms (allow_unqualified_variant=true), se o próximo
                // token pode iniciar um pattern, tratamos como variante
                // desqualificada com payload: `Ok v` → Variant{enum_name:"", variant:"Ok", payload:[v]}.
                // O typeck resolve enum_name via EnumRegistry do scrutinee.
                //
                // Sem sub-pattern following, continua como Pattern::Ident —
                // o typeck resolve variantes unitárias (True, False, None) via EnumRegistry.
                if allow_unqualified_variant && self.can_start_pattern() {
                    let sub_pat = self.parse_pattern_inner(allow_unqualified_variant)?;
                    let end_span = sub_pat.span;
                    let span = start.cover(end_span);
                    return Ok(Spanned::new(
                        Pattern::Variant {
                            enum_name: String::new(),
                            variant: name,
                            payload: Some(vec![sub_pat]),
                        },
                        span,
                    ));
                }
                Ok(Spanned::new(Pattern::Ident(name), start))
            }
            // Literais → Literal pattern
            Token::IntLit(s) => {
                self.advance();
                Ok(Spanned::new(
                    Pattern::Literal(Spanned::new(Expr::IntLit { text: s }, start)),
                    start,
                ))
            }
            Token::FloatLit(s) => {
                self.advance();
                Ok(Spanned::new(
                    Pattern::Literal(Spanned::new(Expr::FloatLit { text: s }, start)),
                    start,
                ))
            }
            Token::TextLit(s) => {
                self.advance();
                Ok(Spanned::new(
                    Pattern::Literal(Spanned::new(Expr::TextLit { text: s }, start)),
                    start,
                ))
            }
            // `()` → Unit literal pattern
            Token::LParen => self.parse_tuple_pattern(start, allow_unqualified_variant),
            // `[]` ou `[h : t]` → Cons pattern (stub )
            Token::LBracket => self.parse_cons_pattern(start),
            _ => Err(self.error("pattern")),
        }
    }

    /// Parse `(p1, p2, ...)` → Tuple pattern.
    /// `()` → Tuple vazia (ou Unit — typeck decide).
    /// `(p)` sem vírgula → desembrulhar (Grouping), igual a expressões.
    /// `(p,)` com trailing comma → Tuple de 1 elemento.
    fn parse_tuple_pattern(
        &mut self,
        start: kata_ast::Span,
        allow_unqualified_variant: bool,
    ) -> Result<Spanned<Pattern>, FrontendError> {
        self.expect(&Token::LParen, "`(`")?;

        // `()` → Tuple vazio
        if matches!(self.peek(), Token::RParen) {
            self.advance();
            return Ok(Spanned::new(Pattern::Tuple(Vec::new()), start));
        }

        let mut elements = vec![self.parse_pattern_inner(allow_unqualified_variant)?];
        let mut had_comma = false;
        while matches!(self.peek(), Token::Comma) {
            had_comma = true;
            self.advance();
            if matches!(self.peek(), Token::RParen) {
                break; // trailing comma
            }
            elements.push(self.parse_pattern_inner(allow_unqualified_variant)?);
        }

        self.expect(&Token::RParen, "`)`")?;
        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);

        // `(p)` sem vírgula → desembrulhar (Grouping).
        // Parênteses só implicam em tupla quando há vírgula.
        if elements.len() == 1 && !had_comma {
            return Ok(Spanned::new(elements.pop().unwrap().node, span));
        }

        Ok(Spanned::new(Pattern::Tuple(elements), span))
    }

    /// `[h : t]` → Cons pattern, `[]` → Nil pattern.
    fn parse_cons_pattern(
        &mut self,
        start: kata_ast::Span,
    ) -> Result<Spanned<Pattern>, FrontendError> {
        self.expect(&Token::LBracket, "`[`")?;

        // `[]` → Nil pattern (lista vazia). Codegen testa val == 0.
        if matches!(self.peek(), Token::RBracket) {
            self.advance();
            return Ok(Spanned::new(Pattern::Nil, start));
        }

        // `[h : t]` → Cons
        let head = self.parse_pattern()?;
        self.expect(&Token::Colon, "`:` (cons pattern)")?;
        let tail = self.parse_pattern()?;
        self.expect(&Token::RBracket, "`]`")?;
        let span = start.cover(tail.span);
        Ok(Spanned::new(
            Pattern::Cons {
                head: Box::new(head),
                tail: Box::new(tail),
            },
            span,
        ))
    }

    /// Parse múltiplos patterns separados por espaço (argumentos de lambda).
    pub(crate) fn parse_patterns(&mut self) -> Result<Vec<Spanned<Pattern>>, FrontendError> {
        let mut patterns = Vec::new();
        // Pelo menos 1 pattern
        patterns.push(self.parse_pattern()?);
        while self.can_start_pattern() {
            patterns.push(self.parse_pattern()?);
        }
        Ok(patterns)
    }

    /// Parse `arity` patterns com aridade-consciência.
    ///
    /// Primeiras `arity-1` posições usam `parse_pattern` (não permite variante
    /// desqualificada com payload — evita ambiguidade com múltiplos args).
    /// A última posição usa `parse_match_pattern` (permite `Some True` como
    /// UM pattern Variant{Some, [True]} — mesmo parser de match arms).
    ///
    /// PRD-exaustividade-aninhada §5.3: `lambda Some True:` em função de
    /// 1 param (arity=1) parseia como UM pattern, não dois.
    pub(crate) fn parse_patterns_arity(
        &mut self,
        arity: usize,
    ) -> Result<Vec<Spanned<Pattern>>, FrontendError> {
        let mut patterns = Vec::new();
        if arity <= 1 {
            // 0 ou 1 param: tudo na última posição → match_pattern
            patterns.push(self.parse_match_pattern()?);
            while self.can_start_pattern() {
                // Se já parseamos arity patterns, extras são erro de aridade
                // (caught by bound-check em check_patterns).
                patterns.push(self.parse_match_pattern()?);
            }
        } else {
            // Primeiras arity-1 com parse_pattern
            for _ in 0..arity - 1 {
                patterns.push(self.parse_pattern()?);
            }
            // Última com parse_match_pattern
            patterns.push(self.parse_match_pattern()?);
            // Extras (erro de aridade, caught por bound-check)
            while self.can_start_pattern() {
                patterns.push(self.parse_pattern()?);
            }
        }
        Ok(patterns)
    }

    /// Verifica se o token atual pode iniciar um pattern.
    pub(crate) fn can_start_pattern(&self) -> bool {
        matches!(
            self.peek(),
            Token::Ident(_)
                | Token::IntLit(_)
                | Token::FloatLit(_)
                | Token::TextLit(_)
                | Token::LParen
                | Token::LBracket
        )
    }
}
