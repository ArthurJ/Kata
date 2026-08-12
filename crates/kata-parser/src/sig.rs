//! Signature parsing — `parse_sig`, `parse_sig_clauses`, `parse_constant_decl`.
//!
//! Extraído de `declarations.rs` — separa a responsabilidade de parsing
//! de assinaturas (nome :: params => ret, cláusulas lambda, constant)
//! do dispatch de top-level items.

use kata_ast::{Directive, Expr, Item, LambdaClause, Spanned, Token, TypeExpr};
use kata_diagnostics::FrontendError;

use crate::CasingPattern;
use crate::Parser;
use crate::expressions::parse_expr;
use crate::is_snake_case;

impl Parser {
    /// Tenta parsear um param de tipo nomeado: `(ident::TypeExpr)`.
    ///
    /// Retorna `Some((name, TypeExpr))` se a posição atual é `(` seguido de
    /// `Ident` snake_case, `::`, e um `TypeExpr`, fechado por `)`. Caso
    /// contrário (incluindo `(T)` como grouping, `(Array::A)` como ParamApp,
    /// `(A -> B)` como Func, ou qualquer outro uso de parênteses em tipos`),
    /// não consome tokens e retorna `None` — o caller deve chamar
    /// `parse_type_expr` normalmente.
    ///
    /// Critério de ambiguidade: só ativa se o `Ident` dentro dos parênteses
    /// é snake_case (primeiro char lowercase). Tipos são PascalCase ou
    /// ALL_CAPS — `Array`, `List`, `Int`, `A`, `K` — nunca snake_case.
    /// Isto elimina a ambiguidade com `(Array::A)` sem lookahead extra.
    pub(crate) fn try_parse_named_type_param(&mut self) -> Option<(String, Spanned<TypeExpr>)> {
        // Precisa de pelo menos: ( Ident :: ... )
        if !matches!(self.peek(), Token::LParen) {
            return None;
        }
        let lookahead = self.pos + 1;
        let t1 = self.tokens.get(lookahead)?;
        let Token::Ident(name) = &t1.token else {
            return None;
        };
        // Só snake_case ativa o caminho nomeado.
        if !is_snake_case(name) {
            return None;
        }
        // Próximo deve ser `::`.
        let t2 = self.tokens.get(lookahead + 1)?;
        if !matches!(t2.token, Token::DoubleColon) {
            return None;
        }
        // Snapshot da posição para rollback se não fechar com `)`.
        let saved_pos = self.pos;

        // Consome `(`.
        self.advance();
        // Consome Ident (nome do param).
        let pname = match self.peek() {
            Token::Ident(s) => s.clone(),
            _ => {
                self.pos = saved_pos;
                return None;
            }
        };
        self.advance();
        // Consome `::`.
        self.advance();
        // Parseia o TypeExpr.
        let ty = match self.parse_type_expr() {
            Ok(t) => t,
            Err(_) => {
                self.pos = saved_pos;
                return None;
            }
        };
        // Precisa fechar `)`.
        if !matches!(self.peek(), Token::RParen) {
            self.pos = saved_pos;
            return None;
        }
        self.advance(); // consome `)`
        Some((pname, ty))
    }

    pub(crate) fn parse_sig(&mut self, directives: Vec<Directive>) -> Result<Item, FrontendError> {
        // name :: T1 T2 ... => TRet
        let name = match self.peek() {
            Token::Ident(s) => {
                let span = self.peek_span();
                let n = s.clone();
                self.advance();
                // Validar casing apenas para nomes alfabéticos (não símbolos como +, -, *)
                if n.chars().next().is_some_and(|c| c.is_alphabetic()) {
                    self.validate_name(&n, CasingPattern::SnakeCase, span)?;
                }
                n
            }
            _ => return Err(self.error("signature name")),
        };
        self.expect(&Token::DoubleColon, "`::`")?;

        // Parse type params until `=>`.
        //
        // Cada param pode ser nomeado: `(x::Type)` — Ident snake_case seguido
        // de `::` e TypeExpr, tudo entre parênteses. Params sem nome: `Type`
        // direto (ex: `Int`, `Float`, `(A -> B)`, `List::A`).
        //
        // Ambiguidade: `(Array::A)` é ParamApp (tipo genérico), não param
        // nomeado. Resolução: o Ident dentro dos parênteses é snake_case
        // (param nomeado) ou PascalCase/ALL_CAPS (tipo). Só snake_case
        // seguido de `::` ativa o caminho de param nomeado.
        let mut params = Vec::new();
        let mut param_names = Vec::new();
        while !matches!(self.peek(), Token::FatArrow | Token::Eof) {
            if let Some((pname, ty)) = self.try_parse_named_type_param() {
                params.push(ty);
                param_names.push(Some(pname));
            } else {
                params.push(self.parse_type_expr()?);
                param_names.push(None);
            }
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
            param_names,
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
    /// Terminação: consome lambdas até encontrar um token que não seja
    /// `Lambda` nem `StmtSep` (nova assinatura, action, diretiva, EOF, etc.).
    pub(crate) fn parse_sig_clauses(
        &mut self,
    ) -> Result<Vec<Spanned<LambdaClause>>, FrontendError> {
        let mut clauses = Vec::new();
        loop {
            // Skip StmtSep entre cláusulas
            while matches!(self.peek(), Token::StmtSep) {
                self.advance();
            }
            // Fim: EOF, Dedent (contexto implements), ou token não-lambda
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
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
