//! Lambda — parse_lambda, parse_lambda_body_block, guard clauses, with bindings.

use kata_ast::{Expr, GuardClause, Spanned, Token, WithBinding};
use kata_diagnostics::FrontendError;

use crate::Parser;
use crate::expressions::parse_expr;

impl Parser {
    /// Parse `lambda <patterns>: <body>` — lambda anônimo (cláusula única).
    ///
    /// Body é expressão única após `:` (sem guards, sem with).
    /// Body pode ser bloco indentado com guard clauses + with.
    pub(crate) fn parse_lambda(&mut self) -> Result<Spanned<Expr>, FrontendError> {
        let start = self.peek_span();
        self.expect(&Token::Lambda, "`lambda`")?;

        // Parse patterns (1 ou mais, separados por espaço)
        let patterns = self.parse_patterns()?;

        // Expect `:`
        self.expect(&Token::Colon, "`:` após patterns do lambda")?;

        // Body é uma expressão única (sem guards).
        // Se há INDENT após `:`, é bloco com guards + with.
        let body = if matches!(self.peek(), Token::Indent) {
            // Bloco indentado — guards + with.
            // parse_lambda_body_block consome o INDENT e retorna
            // o Lambda completo com guards/with preenchidos.
            return self.parse_lambda_body_block(start, patterns);
        } else {
            // Expressão única na mesma linha
            parse_expr(self)?
        };

        let span = start.cover(body.span);
        Ok(Spanned::new(
            Expr::Lambda {
                patterns,
                body: Box::new(body),
                guards: Vec::new(),
                with_bindings: Vec::new(),
            },
            span,
        ))
    }

    /// Parse o bloco indentado de guards dentro de um lambda body.
    ///
    /// Sintaxe:
    /// ```text
    /// lambda x:
    ///     > x 0: x              ← guard clause
    ///     otherwise: - 0 x      ← otherwise
    ///     with                   ← with block (optional, after guards)
    ///         y := + x 1
    /// ```
    ///
    /// Ou, sem guards (apenas body + with opcional):
    /// ```text
    /// lambda [pivo:resto]:
    ///     + (quicksort menores) [pivo : (quicksort maiores)]
    ///     with
    ///         menores := filter (< _ pivo) resto
    /// ```
    ///
    /// Retorna um `Expr::Lambda` com guards e with_bindings preenchidos.
    /// O `body` é preenchido com o body do último guard (ou otherwise) como fallback,
    /// ou com o body direto quando não há guards.
    pub(crate) fn parse_lambda_body_block(
        &mut self,
        start: kata_ast::Span,
        patterns: Vec<Spanned<kata_ast::Pattern>>,
    ) -> Result<Spanned<Expr>, FrontendError> {
        self.expect(&Token::Indent, "INDENT (guards do lambda)")?;

        let mut guards = Vec::new();
        let mut with_bindings = Vec::new();
        let mut last_body = None;
        // Acumula expressões em body direto (sem guards) para produzir Expr::Block
        // quando há múltiplas. Antes, só a última sobrevivia — bug do `let` em lambdas.
        let mut body_stmts: Vec<Spanned<Expr>> = Vec::new();

        loop {
            // Skip StmtSep entre guard clauses
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }

            // `with` block — aparece depois dos guards (ou do body direto)
            if matches!(self.peek(), Token::With) {
                self.advance(); // consume `with`
                self.expect(&Token::Indent, "INDENT (with bindings)")?;
                loop {
                    while matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    if matches!(self.peek(), Token::Dedent | Token::Eof) {
                        break;
                    }
                    let wb = self.parse_with_binding()?;
                    with_bindings.push(wb);
                }
                self.expect(&Token::Dedent, "DEDENT (fim do with)")?;
                continue;
            }

            // `otherwise:` — guard sem condição
            if matches!(self.peek(), Token::Otherwise) {
                let guard = self.parse_guard_clause()?;
                last_body = Some(guard.body.clone());
                guards.push(guard);
                continue;
            }

            // Não é `with` nem `otherwise`. Pode ser:
            //   - guard clause: `expr: body` (condição seguida de `:`)
            //   - body direto: `expr` (sem `:` — só quando não há guards)
            let cond = parse_expr(self)?;
            if matches!(self.peek(), Token::Colon) {
                // Guard clause com condição
                self.advance(); // consome `:`
                let guard_body = parse_expr(self)?;
                if matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                last_body = Some(guard_body.clone());
                guards.push(GuardClause {
                    condition: Some(cond),
                    body: guard_body,
                });
            } else {
                // Body direto sem guards — expressão (seguida de `with` opcional).
                // Acumula em body_stmts; se houver múltiplas, vira Expr::Block.
                if matches!(self.peek(), Token::StmtSep) {
                    self.advance();
                }
                body_stmts.push(cond);
            }
        }

        self.expect(&Token::Dedent, "DEDENT (fim dos guards)")?;

        // body é o último guard body (fallback) ou body direto quando não há guards.
        // Se há múltiplas expressões em body direto (sem guards), produz Expr::Block.
        let body = if !guards.is_empty() {
            last_body.unwrap_or_else(|| Spanned::new(Expr::Unit, start))
        } else if body_stmts.len() > 1 {
            let span = body_stmts
                .first()
                .zip(body_stmts.last())
                .map(|(f, l)| f.span.cover(l.span))
                .unwrap_or(start);
            Spanned::new(Expr::Block { stmts: body_stmts }, span)
        } else if body_stmts.len() == 1 {
            body_stmts.pop().unwrap()
        } else {
            Spanned::new(Expr::Unit, start)
        };

        let end_span = self
            .tokens
            .get(self.pos - 1)
            .map(|t| t.span)
            .unwrap_or(start);
        let span = start.cover(end_span);
        Ok(Spanned::new(
            Expr::Lambda {
                patterns,
                body: Box::new(body),
                guards,
                with_bindings,
            },
            span,
        ))
    }

    /// Parse uma guard clause: `> expr: body` ou `otherwise: body`.
    fn parse_guard_clause(&mut self) -> Result<GuardClause, FrontendError> {
        let condition = if matches!(self.peek(), Token::Otherwise) {
            self.advance(); // consume `otherwise`
            None
        } else {
            // `>` é o token que inicia a condição do guard
            // Mas `>` pode ser um identificador (operador prefixo `>`)
            // O PRD usa `> x 0: x` — `>` é o callee de uma Apply
            // Na notação prefixa, `> x 0` é `Expr::Apply { >, [x, 0] }`
            // Parser: parse_expr greedy consome `> x 0` como Apply
            let cond = parse_expr(self)?;
            Some(cond)
        };

        self.expect(&Token::Colon, "`:` após guard condition")?;
        let body = parse_expr(self)?;

        // Consume trailing StmtSep
        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(GuardClause { condition, body })
    }

    /// Parse um binding do `with` block: `nome := expr`.
    pub(crate) fn parse_with_binding(&mut self) -> Result<WithBinding, FrontendError> {
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("binding name in `with`")),
        };
        self.expect(&Token::BindAssign, "`:=` in with binding")?;
        let value = parse_expr(self)?;

        // Consume trailing StmtSep
        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(WithBinding { name, value })
    }
}
