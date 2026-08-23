//! Signature parsing — `parse_sig`, `parse_sig_clauses`, `parse_constant_decl`.
//!
//! Extraído de `declarations.rs` — separa a responsabilidade de parsing
//! de assinaturas (nome :: params => ret, cláusulas lambda, constant)
//! do dispatch de top-level items.

use kata_ast::{Directive, Expr, Item, LambdaClause, SelectArm, Spanned, Token, WithBinding};
use kata_diagnostics::FrontendError;

use crate::CasingPattern;
use crate::Parser;
use crate::expressions::parse_expr;
use crate::is_snake_case;

impl Parser {
    pub(crate) fn parse_sig(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        // name :: T1 T2 ... => TRet
        let name = match self.peek() {
            Token::Ident(s) => {
                let span = self.peek_span();
                let n = s.clone();
                self.advance();
                self.validate_name(&n, CasingPattern::SnakeCase, span)?;
                n
            }
            _ => return Err(self.error("signature name")),
        };
        self.expect(&Token::DoubleColon, "`::`")?;

        // Parse type params until `=>`. Funções puras são exclusivamente
        // posicionais — cada param é um TypeExpr direto.
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::FatArrow | Token::Eof) {
            params.push(self.parse_type_expr()?);
        }

        self.expect(&Token::FatArrow, "`=>`")?;
        let ret = self.parse_type_expr()?;

        // Cláusulas lambda após assinatura (função nomeada com corpo Kata).
        // Lambda no mesmo nível da assinatura: se o próximo token é `lambda`,
        // parsear cláusulas. Se não, body = None (FFI — corpo suprido por @ffi).
        // Consumir StmtSep antes de checar (newline entre assinatura e lambda).
        while matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }
        let body = if matches!(self.peek(), Token::Lambda) {
            Some(self.parse_sig_clauses()?)
        } else {
            None
        };

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

    /// Parse cláusulas lambda no mesmo nível da assinatura.
    ///
    /// ```text
    /// fat :: Int Int => Int
    /// lambda 0 acc: acc
    /// lambda n acc: fat (- n 1) (* n acc)
    /// ```
    ///
    /// Suporta `with` cross-clause: um bloco `with` no mesmo nível das
    /// cláusulas (após a última) cujos bindings são injetados em todas as
    /// cláusulas como açúcar sintático.
    ///
    /// ```text
    /// quicksort :: [Int] => [Int]
    /// lambda []: []
    /// lambda [pivo:resto]:
    ///     + (quicksort menores) [pivo : (quicksort maiores)]
    /// with
    ///     menores := filter (< _ pivo) resto
    ///     maiores := filter (>= _ pivo) resto
    /// ```
    ///
    /// Terminação: consome lambdas até encontrar um token que não seja
    /// `Lambda` nem `StmtSep` (nova assinatura, action, diretiva, EOF, etc.).
    /// Se o token for `With`, parseia os bindings cross-clause e os injeta
    /// em todas as cláusulas já parseadas.
    pub(crate) fn parse_sig_clauses(
        &mut self,
    ) -> Result<Vec<Spanned<LambdaClause>>, FrontendError> {
        let mut clauses: Vec<Spanned<LambdaClause>> = Vec::new();
        loop {
            // Skip StmtSep entre cláusulas
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            // Fim: EOF, Dedent (contexto implements), ou token não-lambda
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            // `with` cross-clause — bindings compartilhados entre todas as cláusulas
            if matches!(self.peek(), Token::With) {
                let cross_bindings = self.parse_cross_clause_with()?;
                // Injetar cada binding cross-clause só nas cláusulas cujo
                // body/guards referenciam o nome do binding. Isso evita erros
                // de "variável não vinculada" em cláusulas com patterns
                // diferentes (ex: `lambda []: []` não referencia `resto`).
                // Bindings cross-clause vêm antes dos bindings aninhados.
                for clause in &mut clauses {
                    let applicable: Vec<WithBinding> = cross_bindings
                        .iter()
                        .filter(|wb| clause_uses_name(&clause.node, &wb.name))
                        .cloned()
                        .collect();
                    if !applicable.is_empty() {
                        let mut combined = applicable;
                        combined.append(&mut clause.node.with_bindings);
                        clause.node.with_bindings = combined;
                    }
                }
                // Após o `with` cross-clause, não há mais cláusulas.
                break;
            }
            if !matches!(self.peek(), Token::Lambda) {
                break;
            }

            let clause_start = self.peek_span();
            let clause = self.parse_lambda_clause()?;
            let span = clause_start.cover(clause.body.span);
            clauses.push(Spanned::new(clause, span));
        }

        Ok(clauses)
    }

    /// Parse um bloco `with` cross-clause: `with` seguido de bindings indentados.
    /// Retorna os bindings para serem injetados em todas as cláusulas.
    fn parse_cross_clause_with(&mut self) -> Result<Vec<WithBinding>, FrontendError> {
        self.expect(&Token::With, "`with` cross-clause")?;
        self.expect(&Token::Indent, "INDENT (with bindings cross-clause)")?;
        let mut bindings = Vec::new();
        loop {
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            let wb = self.parse_with_binding()?;
            bindings.push(wb);
        }
        self.expect(&Token::Dedent, "DEDENT (fim do with cross-clause)")?;
        Ok(bindings)
    }

    /// Parse uma cláusula lambda: `lambda <patterns>: <body>`.
    /// Body é expressão única (sem guards, sem with).
    /// Body pode ser bloco indentado com guards + with.
    fn parse_lambda_clause(&mut self) -> Result<LambdaClause, FrontendError> {
        self.expect(&Token::Lambda, "`lambda`")?;

        // Parse patterns (1 ou mais, separados por espaço)
        let patterns = self.parse_patterns()?;

        // Expect `:`
        self.expect(&Token::Colon, "`:` após patterns da cláusula")?;

        // Body é expressão única (sem guards).
        // Se há INDENT após `:`, é bloco com guards + with.
        if matches!(self.peek(), Token::Indent) {
            // Bloco indentado — guards + with.
            // Reusar parse_lambda_body_block do expressions module.
            // Precisamos extrair guards e with_bindings do Lambda retornado.
            let clause_start = self.peek_span();
            let lambda_expr = self.parse_lambda_body_block(clause_start, patterns)?;
            // Desempacotar o Lambda para extrair guards e with_bindings
            match lambda_expr.node {
                Expr::Lambda {
                    patterns,
                    body,
                    guards,
                    with_bindings,
                } => {
                    // Consume trailing StmtSep
                    if matches!(self.peek(), Token::StmtSep) {
                        self.advance();
                    }
                    return Ok(LambdaClause {
                        patterns,
                        body: *body,
                        guards,
                        with_bindings,
                    });
                }
                other => panic!("parse_lambda_body_block returned {other:?}, expected Lambda"),
            }
        }

        let body = parse_expr(self)?;

        // Detectar `with` same-line (erro comum): após a expressão,
        // se há INDENT seguido de `with`, o usuário tentou usar `with`
        // no path same-line — que não é suportado. Binding local sem
        // guards deve usar `let` no body indentado.
        if matches!(self.peek(), Token::Indent) {
            let saved_pos = self.pos;
            self.advance();
            if matches!(self.peek(), Token::With) {
                return Err(self.error(
                    "`with` same-line não é suportado — use `let` no body indentado, ou `with` no path indentado (lambda x:\\n    expr\\n    with ...)",
                ));
            }
            self.pos = saved_pos;
        }

        // Consume trailing StmtSep
        if matches!(self.peek(), Token::StmtSep) {
            self.advance();
        }

        Ok(LambdaClause {
            patterns,
            body,
            guards: Vec::new(),
            with_bindings: Vec::new(),
        })
    }

    /// Parse `constant nome := expr` ou `constant _ := expr`.
    /// Top-level only. Diretivas são aceitas mas ignoradas por ora
    /// (diferentes de @comptime que era o antigo mecanismo).
    pub(crate) fn parse_constant_decl(
        &mut self,
        _directives: Vec<Directive>,
    ) -> Result<Item, FrontendError> {
        self.expect(&Token::Constant, "`constant`")?;

        // Nome do binding (ou `_` para descarte).
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("nome do binding após `constant`")),
        };

        // Validar casing: constant é snake_case.
        if !is_snake_case(&name) {
            return Err(self.error(&format!(
                "nome de `constant` \"{name}\" deve ser snake_case (minúsculo)"
            )));
        }

        self.expect(&Token::BindAssign, "`:=` após nome da constant")?;
        let value = crate::expressions::parse_expr(self)?;

        Ok(Item::ConstantDecl { name, value })
    }
}

// ── Cross-clause with helpers ─────────────────────────────────────

/// Verifica se uma cláusula referencia `name` no body ou em algum guard.
fn clause_uses_name(clause: &LambdaClause, name: &str) -> bool {
    expr_uses_name(&clause.body, name)
        || clause.guards.iter().any(|g| {
            g.condition
                .as_ref()
                .is_some_and(|c| expr_uses_name(c, name))
                || expr_uses_name(&g.body, name)
        })
        || clause
            .with_bindings
            .iter()
            .any(|wb| wb.name == name || expr_uses_name(&wb.value, name))
}

/// Walker recursivo: verifica se `name` aparece como `Expr::Ident` em `expr`.
fn expr_uses_name(expr: &Spanned<Expr>, name: &str) -> bool {
    match &expr.node {
        Expr::Ident { name: n } => n == name,
        Expr::Apply { callee, args } => {
            expr_uses_name(callee, name) || args.iter().any(|a| expr_uses_name(a, name))
        }
        Expr::TypeAscription { expr, .. } => expr_uses_name(expr, name),
        Expr::Grouping { inner } => expr_uses_name(inner, name),
        Expr::Tuple { elements } => elements.iter().any(|e| expr_uses_name(e, name)),
        Expr::Let { name: n, value } => n == name || expr_uses_name(value, name),
        Expr::LetDestruct { names, value } => {
            names.iter().any(|n| n == name) || expr_uses_name(value, name)
        }
        Expr::VariantQual { .. } => false,
        Expr::Lambda {
            body,
            guards,
            with_bindings,
            ..
        } => {
            expr_uses_name(body, name)
                || guards.iter().any(|g| {
                    g.condition
                        .as_ref()
                        .is_some_and(|c| expr_uses_name(c, name))
                        || expr_uses_name(&g.body, name)
                })
                || with_bindings
                    .iter()
                    .any(|wb| wb.name == name || expr_uses_name(&wb.value, name))
        }
        Expr::Match { scrutinee, arms } => {
            expr_uses_name(scrutinee, name)
                || arms.iter().any(|a| {
                    a.guard.as_ref().is_some_and(|g| expr_uses_name(g, name))
                        || expr_uses_name(&a.body, name)
                })
        }
        Expr::Hole => false,
        Expr::Pipe { lhs, rhs } | Expr::PipeFallback { lhs, rhs } => {
            expr_uses_name(lhs, name) || expr_uses_name(rhs, name)
        }
        Expr::PipeLimit { lhs, rhs, limit } => {
            expr_uses_name(lhs, name) || expr_uses_name(rhs, name) || expr_uses_name(limit, name)
        }
        Expr::ActionCall { args, .. } => expr_uses_name(args, name),
        Expr::TypeOf { expr } => expr_uses_name(expr, name),
        Expr::Return(e) => expr_uses_name(e, name),
        Expr::Loop { body } => body.iter().any(|e| expr_uses_name(e, name)),
        Expr::Break | Expr::Continue | Expr::Unit => false,
        Expr::Var { name: n, value } => n == name || expr_uses_name(value, name),
        Expr::Reassign { name: n, value } => n == name || expr_uses_name(value, name),
        Expr::Question(e) => expr_uses_name(e, name),
        Expr::DotAccess { expr, .. } => expr_uses_name(expr, name),
        Expr::ListLit { elements } => elements.iter().any(|e| expr_uses_name(e, name)),
        Expr::ArrayLit { elements } => elements.iter().any(|e| expr_uses_name(e, name)),
        Expr::DictLit { entries } => entries
            .iter()
            .any(|(k, v)| expr_uses_name(k, name) || expr_uses_name(v, name)),
        Expr::SetLit { elements } => elements.iter().any(|e| expr_uses_name(e, name)),
        Expr::RangeLit {
            start, step, end, ..
        } => expr_uses_name(start, name) || expr_uses_name(step, name) || expr_uses_name(end, name),
        Expr::ForIn {
            var_name,
            iterable,
            body,
        } => {
            var_name == name
                || expr_uses_name(iterable, name)
                || body.iter().any(|e| expr_uses_name(e, name))
        }
        Expr::In { item, collection } => {
            expr_uses_name(item, name) || expr_uses_name(collection, name)
        }
        Expr::ChannelSend { channel, value } => {
            expr_uses_name(channel, name) || expr_uses_name(value, name)
        }
        Expr::ChannelRecv { channel, .. } => expr_uses_name(channel, name),
        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            arms.iter().any(|a| match a {
                SelectArm::Channel {
                    channel,
                    bind_name,
                    body,
                } => {
                    expr_uses_name(channel, name) || bind_name == name || expr_uses_name(body, name)
                }
                SelectArm::IoRead {
                    handle_expr,
                    bind_name,
                    body,
                    ..
                } => {
                    expr_uses_name(handle_expr, name)
                        || bind_name == name
                        || expr_uses_name(body, name)
                }
            }) || timeout_ms.as_ref().is_some_and(|t| expr_uses_name(t, name))
                || timeout_body
                    .as_ref()
                    .is_some_and(|t| expr_uses_name(t, name))
        }
        Expr::Block { stmts } => stmts.iter().any(|e| expr_uses_name(e, name)),
        // Literais não contêm idents
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::TextLit { .. }
        | Expr::BytesLit { .. } => false,
    }
}
