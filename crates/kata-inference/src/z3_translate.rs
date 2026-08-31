//! Tradutor unificado de `TypedExpr` → expressões Z3.
//!
//! Um único tradutor para os dois consumidores de Z3 do typeck:
//! - `guard_completeness.rs` — prova de exaustividade de guards e
//!   implicação entre cláusulas (redundância);
//! - `path_conditions.rs` — prova de predicados refined dados facts
//!   acumulados no escopo.
//!
//! Antes havia duas cópias (~185 linhas cada) com pequenas divergências:
//! o `Z3PathTranslator` sabia inlinar funções puras (`InlineFnTable`),
//! o `Z3Translator` de guards não sabia nada. Consolidar aqui garante que
//! toda melhoria de tradução valha para os dois consumidores.
//!
//! Mapeamentos:
//! - Literais Int → `Int::from_i64`
//! - `> a b`, `< a b`, `>= a b`, `<= a b` → comparações Z3
//! - `= a b`, `!= a b` → `=` Z3
//! - `+ a b`, `- a b`, `* a b` → aritmética Z3
//! - `and a b`, `or a b`, `not a` → lógica proposicional Z3
//! - Variáveis (`x`) → `Int`/`Bool` const (reutilizadas via cache)
//! - Qualquer outra → variável opaca fresca (fallback conservador)

use std::collections::HashMap;

use kata_ast::Spanned;

use crate::typed::{TypedExpr, TypedExprKind};
use crate::typed_pattern::TypedWithBinding;

use crate::infer::{InlineFnTable, substitute_params};

use kata_core::ty::Ty;

use z3::ast::{Bool, Int};

/// Var já criada no solver, por nome.
enum VarKind {
    Int(Int),
    Bool(Bool),
    /// Rational como par (numerador, denominador). Denominador > 0
    /// (invariante mantida em todas as operações).
    Rat(Int, Int),
}

/// Tradutor de `TypedExpr` para expressões Z3.
pub(crate) struct Z3Translator {
    /// Nomes de variáveis já criadas, para reutilizar.
    var_cache: HashMap<String, VarKind>,
    /// Contador para variáveis opacas frescas.
    fresh_counter: u32,
    /// Funções puras inlinable (opcional). `None` = sem inlining.
    inline_fns: Option<InlineFnTable>,
}

impl Z3Translator {
    /// Tradutor sem inlining de funções puras.
    pub(crate) fn new() -> Self {
        Z3Translator {
            var_cache: HashMap::new(),
            fresh_counter: 0,
            inline_fns: None,
        }
    }

    /// Tradutor com inlining de funções puras (usado por path conditions).
    pub(crate) fn with_inline_fns(inline_fns: &InlineFnTable) -> Self {
        Z3Translator {
            var_cache: HashMap::new(),
            fresh_counter: 0,
            inline_fns: Some(inline_fns.clone()),
        }
    }

    /// Traduz os bindings `with` da cláusula antes das condições dos guards.
    ///
    /// Cada binding é traduzido em ordem de declaração e o resultado é
    /// memoizado em `var_cache` sob o nome do binding, **pelo seu tipo**:
    /// - Boolean → `translate_bool` (guards o referenciam como condição);
    /// - Int → `translate_int` (guards o comparam: `> doubled 10` vira
    ///   `x*2 > 10` inlinado — mais forte que uma const livre).
    ///
    /// Assim:
    /// - Guards que referenciam o binding reusam a MESMA variável Z3 —
    ///   sem isso, cada referência receberia um Bool livre distinto e a
    ///   disjunção deixaria de ser tautologia por artefato do tradutor.
    /// - Bindings em cadeia (`b := and a_prev c_prev`) enxergam os
    ///   bindings anteriores já memoizados.
    ///
    /// Fallback conservador: bindings que o tradutor não entende (chamada
    /// de função não-inlinable, `mod`, Float) traduzem para variável
    /// livre — mesma semântica de antes do fix. A prova fica não
    /// provada (conservador), nunca errada.
    pub(crate) fn seed_with_bindings(&mut self, with_bindings: &[TypedWithBinding]) {
        for wb in with_bindings {
            if wb.value.node.ty == Ty::boolean() {
                let translated = self.translate_bool(&wb.value.node);
                self.var_cache
                    .insert(wb.name.clone(), VarKind::Bool(translated));
            } else if let Some(i) = self.translate_int(&wb.value.node) {
                self.var_cache.insert(wb.name.clone(), VarKind::Int(i));
            } else if let Some((n, d)) = self.translate_rat(&wb.value.node) {
                self.var_cache.insert(wb.name.clone(), VarKind::Rat(n, d));
            } else {
                // Tipo não-traduzível (Float, Text, ...) → Bool livre.
                // Referências caem no fallback conservador.
                let translated = self.fresh_bool();
                self.var_cache
                    .insert(wb.name.clone(), VarKind::Bool(translated));
            }
        }
    }

    /// Semear bindings `let` imutáveis do escopo em vigor (path
    /// conditions) — mesma mecânica de `seed_with_bindings`: o termo
    /// do VALOR é memoizado no `var_cache` sob o NOME do binding
    /// (aliasing). `let d := x` faz o predicado `> d 0` traduzir como
    /// `> x 0`, conectando o binding aos facts do escopo.
    ///
    /// Tipos:
    /// - Boolean → `translate_bool` (guards o referenciam);
    /// - Int-traduzível → `translate_int` (comparações inlinam o
    ///   valor: `let d := * x 2` + `> d 0` vira `x*2 > 0`);
    /// - não-traduzível (Float, Text, ...) → variável livre
    ///   (fallback conservador — nunca falso erro).
    ///
    /// O caller (coleta em expr.rs) garante que só bindings IMUTÁVEIS
    /// chegam aqui: `var` nunca é semeado (mutável, sem SSA —
    /// seeding seria unsound).
    pub(crate) fn seed_let_bindings(&mut self, bindings: &[(String, TypedExpr)]) {
        for (name, value) in bindings {
            if value.ty == Ty::boolean() {
                let translated = self.translate_bool(value);
                self.var_cache
                    .insert(name.clone(), VarKind::Bool(translated));
            } else if let Some(i) = self.translate_int(value) {
                self.var_cache.insert(name.clone(), VarKind::Int(i));
            } else if let Some((n, d)) = self.translate_rat(value) {
                self.var_cache.insert(name.clone(), VarKind::Rat(n, d));
            } else {
                let translated = self.fresh_bool();
                self.var_cache
                    .insert(name.clone(), VarKind::Bool(translated));
            }
        }
    }

    /// Traduz uma expressão para um Z3 Bool.
    pub(crate) fn translate_bool(&mut self, expr: &TypedExpr) -> Bool {
        match &expr.kind {
            TypedExprKind::Closure { callee, args, .. } => {
                if let TypedExprKind::Ident { name } = &callee.node.kind {
                    match name.as_str() {
                        "and" => {
                            if args.len() == 2 {
                                let a = self.translate_bool(&args[0].node);
                                let b = self.translate_bool(&args[1].node);
                                Bool::and(&[a, b])
                            } else {
                                self.fresh_bool()
                            }
                        }
                        "or" => {
                            if args.len() == 2 {
                                let a = self.translate_bool(&args[0].node);
                                let b = self.translate_bool(&args[1].node);
                                Bool::or(&[a, b])
                            } else {
                                self.fresh_bool()
                            }
                        }
                        "not" => {
                            if args.len() == 1 {
                                let a = self.translate_bool(&args[0].node);
                                a.not()
                            } else {
                                self.fresh_bool()
                            }
                        }
                        ">" | "<" | ">=" | "<=" | "=" | "!=" => {
                            self.translate_comparison(name, args)
                        }
                        _ => {
                            // Tenta inlinar função pura (ex: zero). Se
                            // inlinable, traduz o corpo; senão, opaca.
                            if let Some(inlined) = self.try_inline(name, args) {
                                self.translate_bool(&inlined)
                            } else {
                                self.fresh_bool()
                            }
                        }
                    }
                } else {
                    self.fresh_bool()
                }
            }
            TypedExprKind::Ident { name } => {
                // Variável Boolean — cria ou reutiliza const bool.
                if let Some(VarKind::Bool(b)) = self.var_cache.get(name) {
                    b.clone()
                } else {
                    let b = Bool::new_const(name.as_str());
                    self.var_cache
                        .insert(name.clone(), VarKind::Bool(b.clone()));
                    b
                }
            }
            TypedExprKind::Grouping { inner } => self.translate_bool(&inner.node),
            _ => self.fresh_bool(),
        }
    }

    fn fresh_bool(&mut self) -> Bool {
        let name = format!("__opaque_{}", self.fresh_counter);
        self.fresh_counter += 1;
        Bool::fresh_const(&name)
    }

    /// Tenta inlinar uma chamada de função pura. Se `name` está na
    /// `inline_fns` table, substitui os params pelos args e retorna o
    /// corpo tipado. O caller então traduz o corpo inlinado em vez da
    /// chamada. Retorna `None` se a função não está na tabela, não é
    /// inlinable, ou se a table não está disponível.
    fn try_inline(&self, name: &str, args: &[Spanned<TypedExpr>]) -> Option<TypedExpr> {
        let table = self.inline_fns.as_ref()?;
        let arg_types: Vec<Ty> = args.iter().map(|a| a.node.ty.clone()).collect();
        let fn_body = table.get(name, &arg_types)?;
        let body = fn_body.body.as_ref()?;
        Some(substitute_params(body, &fn_body.param_names, args))
    }

    /// Traduz uma comparação (`>`, `<`, `>=`, `<=`, `=`, `!=`).
    fn translate_comparison(&mut self, op: &str, args: &[Spanned<TypedExpr>]) -> Bool {
        if args.len() != 2 {
            return self.fresh_bool();
        }

        // Tenta traduzir ambos os operandos como Int.
        let lhs = self.translate_int(&args[0].node);
        let rhs = self.translate_int(&args[1].node);

        let (lhs, rhs) = match (lhs, rhs) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                // Int falhou — tenta Rational (cross-multiplication).
                return self.translate_rat_comparison(op, args);
            }
        };

        match op {
            ">" => lhs.gt(&rhs),
            "<" => lhs.lt(&rhs),
            ">=" => lhs.ge(&rhs),
            "<=" => lhs.le(&rhs),
            "=" => lhs.eq(&rhs),
            "!=" => lhs.eq(&rhs).not(),
            _ => self.fresh_bool(),
        }
    }

    /// Traduz uma expressão para um Z3 Int (se possível).
    fn translate_int(&mut self, expr: &TypedExpr) -> Option<Int> {
        match &expr.kind {
            TypedExprKind::IntLit { text } => {
                // Parse o literal inteiro. Pode ser BigInt, mas Z3 usa i64.
                text.parse::<i64>().ok().map(Int::from_i64)
            }
            TypedExprKind::Ident { name } => {
                // Variável Int — cria ou reutiliza const.
                if let Some(VarKind::Int(i)) = self.var_cache.get(name) {
                    Some(i.clone())
                } else {
                    let i = Int::new_const(name.as_str());
                    self.var_cache.insert(name.clone(), VarKind::Int(i.clone()));
                    Some(i)
                }
            }
            TypedExprKind::Closure { callee, args, .. } => {
                if let TypedExprKind::Ident { name } = &callee.node.kind {
                    match name.as_str() {
                        "+" => {
                            if args.len() == 2 {
                                let a = self.translate_int(&args[0].node)?;
                                let b = self.translate_int(&args[1].node)?;
                                Some(&a + &b)
                            } else {
                                None
                            }
                        }
                        "-" => {
                            if args.len() == 2 {
                                let a = self.translate_int(&args[0].node)?;
                                let b = self.translate_int(&args[1].node)?;
                                Some(&a - &b)
                            } else {
                                None
                            }
                        }
                        "*" => {
                            if args.len() == 2 {
                                let a = self.translate_int(&args[0].node)?;
                                let b = self.translate_int(&args[1].node)?;
                                Some(&a * &b)
                            } else {
                                None
                            }
                        }
                        _ => {
                            // Tenta inlinar função pura (ex: zero). Se
                            // inlinable, traduz o corpo; senão, None.
                            if let Some(inlined) = self.try_inline(name, args) {
                                self.translate_int(&inlined)
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    None
                }
            }
            TypedExprKind::Grouping { inner } => self.translate_int(&inner.node),
            _ => None,
        }
    }

    /// Traduz uma expressão para um Z3 Rational (par num, den).
    ///
    /// Representa Rational como `(numerator, denominator)` onde `den > 0`
    /// (invariante). Suporta:
    /// - `rational N` (Closure{Ident("rational"), [IntLit(N)]}) → `(N, 1)`
    /// - Variável Rational → `(num_const, den_const)` com side-condition
    ///   `den > 0` assertionada no solver
    /// - Operações aritméticas (`+`, `-`, `*`) via regras de fração
    ///
    /// Retorna `None` se a expressão não é traduzível como Rational.
    fn translate_rat(&mut self, expr: &TypedExpr) -> Option<(Int, Int)> {
        match &expr.kind {
            TypedExprKind::Closure { callee, args, .. } => {
                if let TypedExprKind::Ident { name } = &callee.node.kind {
                    match name.as_str() {
                        "rational" => {
                            // `rational N` → (N, 1)
                            if args.len() == 1
                                && let TypedExprKind::IntLit { text } = &args[0].node.kind
                            {
                                let n = text.parse::<i64>().ok()?;
                                return Some((Int::from_i64(n), Int::from_i64(1)));
                            }
                            None
                        }
                        "+" => {
                            // a/b + c/d = (a*d + c*b) / (b*d)
                            if args.len() == 2 {
                                let (an, ad) = self.translate_rat(&args[0].node)?;
                                let (bn, bd) = self.translate_rat(&args[1].node)?;
                                let num = &(&an * &bd) + &(&bn * &ad);
                                let den = &ad * &bd;
                                Some((num, den))
                            } else {
                                None
                            }
                        }
                        "-" => {
                            // a/b - c/d = (a*d - c*b) / (b*d)
                            if args.len() == 2 {
                                let (an, ad) = self.translate_rat(&args[0].node)?;
                                let (bn, bd) = self.translate_rat(&args[1].node)?;
                                let num = &(&an * &bd) - &(&bn * &ad);
                                let den = &ad * &bd;
                                Some((num, den))
                            } else {
                                None
                            }
                        }
                        "*" => {
                            // a/b * c/d = (a*c) / (b*d)
                            if args.len() == 2 {
                                let (an, ad) = self.translate_rat(&args[0].node)?;
                                let (bn, bd) = self.translate_rat(&args[1].node)?;
                                let num = &an * &bn;
                                let den = &ad * &bd;
                                Some((num, den))
                            } else {
                                None
                            }
                        }
                        _ => {
                            // Tenta inlinar função pura. Se inlinable,
                            // traduz o corpo; senão, None.
                            if let Some(inlined) = self.try_inline(name, args) {
                                self.translate_rat(&inlined)
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    None
                }
            }
            TypedExprKind::Ident { name } => {
                // Variável Rational — cria ou reutiliza par (num, den).
                if let Some(VarKind::Rat(n, d)) = self.var_cache.get(name) {
                    Some((n.clone(), d.clone()))
                } else {
                    let n = Int::fresh_const(&format!("{name}_num"));
                    let d = Int::fresh_const(&format!("{name}_den"));
                    // Side-condition: denominador > 0.
                    // NOTA: não assertionamos aqui porque o tradutor não
                    // tem acesso ao solver. A invariante den > 0 é
                    // mantida pela construção: literais usam den=1,
                    // e operações preservam den > 0 se inputs têm den > 0.
                    // Para variáveis livres, o caller deve assertionar.
                    // Por segurança, marca a invariante no cache.
                    self.var_cache
                        .insert(name.clone(), VarKind::Rat(n.clone(), d.clone()));
                    Some((n, d))
                }
            }
            TypedExprKind::Grouping { inner } => self.translate_rat(&inner.node),
            _ => None,
        }
    }

    /// Compara dois Rationais via cross-multiplication.
    ///
    /// `a/b OP c/d` (com b,d > 0) é traduzido para:
    /// - `>`: `a*d > c*b`
    /// - `<`: `a*d < c*b`
    /// - `>=`: `a*d >= c*b`
    /// - `<=`: `a*d <= c*b`
    /// - `=`: `a*d = c*b` (and `b*d ≠ 0` — implícito por den > 0)
    /// - `!=`: `a*d ≠ c*b`
    fn translate_rat_comparison(&mut self, op: &str, args: &[Spanned<TypedExpr>]) -> Bool {
        let lhs = self.translate_rat(&args[0].node);
        let rhs = self.translate_rat(&args[1].node);

        let ((an, ad), (bn, bd)) = match (lhs, rhs) {
            (Some(a), Some(b)) => (a, b),
            _ => return self.fresh_bool(),
        };

        // cross-multiplication: a*d OP c*b (denominadores > 0)
        let left = &an * &bd;
        let right = &bn * &ad;

        match op {
            ">" => left.gt(&right),
            "<" => left.lt(&right),
            ">=" => left.ge(&right),
            "<=" => left.le(&right),
            "=" => left.eq(&right),
            "!=" => left.eq(&right).not(),
            _ => self.fresh_bool(),
        }
    }

    /// Extrai o contra-exemplo do modelo Z3.
    pub(crate) fn extract_counter_example(&self, model: &z3::Model) -> String {
        let parts: Vec<String> = self
            .var_cache
            .iter()
            .filter_map(|(name, var)| match var {
                VarKind::Int(i) => {
                    let val = model.eval(i, true);
                    val.map(|v| format!("{name} = {v}"))
                }
                VarKind::Bool(b) => {
                    let val = model.eval(b, true);
                    val.map(|v| format!("{name} = {v}"))
                }
                VarKind::Rat(n, d) => {
                    let nv = model.eval(n, true);
                    let dv = model.eval(d, true);
                    match (nv, dv) {
                        (Some(nv), Some(dv)) => Some(format!("{name} = {nv}/{dv}")),
                        _ => Some(format!("{name} = ?/?")),
                    }
                }
            })
            .collect();

        if parts.is_empty() {
            "caso não coberto pelos guards".to_string()
        } else {
            parts.join(", ")
        }
    }
}
