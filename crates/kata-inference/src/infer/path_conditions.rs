//! Path conditions — facts acumulados no contexto de inferência.
//!
//! Cada fact é um `TypedExpr` booleano conhecido como verdadeiro no ponto
//! atual do programa (ex: guard de match, condição de braço Boolean).
//! Quando uma ascription-refined é encontrada e `const_eval_predicate`
//! retorna `None` (não-literal), os facts são asserting no Z3 junto com
//! o predicado para provar implicação em compile-time.
//!
//! Nível 1 (PRD-refinement-propagation): guards locais apenas.

use crate::typed::TypedExpr;
use crate::typed::TypedExprKind;

use kata_ast::Spanned;

use z3::{Config, SatResult, Solver, ast::Bool, with_z3_config};

use super::post_conditions::{InlineFnTable, substitute_params};
use kata_core::ty::Ty;

/// Facts acumulados no contexto de inferência.
///
/// Cada fact é um `TypedExpr` booleano verdadeiro no escopo atual.
///
/// Duas lojas de facts:
/// - `facts`: facts de guard/Boolean — braço-específicos, rolled back
///   ao sair do braço de match/lambda.
/// - `learned_facts`: facts da Direção B (aprendidos via dispatch) —
///   trans-escopo, preservados ao sair do braço. Sound porque bindings
///   `let`/`constant` são imutáveis: o conhecimento sobre um binding
///   é válido pelo seu tempo de vida.
///
/// O Z3 vê `facts + learned_facts` concatenados via `facts()`.
/// `checkpoint()`/`rollback_to(n)` gerenciam o escopo: grava-se o
/// índice de `facts` antes do braço, trunca-se ao sair (preservando
/// `learned_facts`).
#[derive(Debug, Clone, Default)]
pub(crate) struct PathConditionCtx {
    facts: Vec<TypedExpr>,
    learned_facts: Vec<TypedExpr>,
}

impl PathConditionCtx {
    /// Adiciona um fact braço-específico (guard/Boolean).
    /// Rolled back ao sair do braço via `rollback_to`.
    pub(crate) fn add_fact(&mut self, fact: TypedExpr) {
        self.facts.push(fact);
    }

    /// Adiciona um fact trans-escopo (Direção B — aprendido via dispatch).
    /// Preservado ao sair do braço.
    pub(crate) fn add_learned_fact(&mut self, fact: TypedExpr) {
        self.learned_facts.push(fact);
    }

    /// Facts acumulados (facts + learned_facts concatenados).
    /// O Z3 vê o conjunto unificado.
    pub(crate) fn facts(&self) -> Vec<&TypedExpr> {
        self.facts.iter().chain(self.learned_facts.iter()).collect()
    }

    /// True se não há facts nem learned_facts.
    pub(crate) fn is_empty(&self) -> bool {
        self.facts.is_empty() && self.learned_facts.is_empty()
    }

    /// Grava o índice atual de `facts` como checkpoint.
    /// Usar antes de entrar num braço; `rollback_to` ao sair.
    pub(crate) fn checkpoint(&self) -> usize {
        self.facts.len()
    }

    /// Trunca `facts` de volta ao checkpoint, preservando `learned_facts`.
    /// Usar ao sair de um braço para remover facts de guard.
    pub(crate) fn rollback_to(&mut self, checkpoint: usize) {
        self.facts.truncate(checkpoint);
    }
}

/// Configura Z3 com rlimit para determinismo de esforço.
/// Igual ao `z3_config` de `guard_completeness.rs`.
fn z3_config() -> Config {
    let mut cfg = Config::new();
    cfg.set_param_value("rlimit", "10000");
    cfg
}

/// Tenta provar que o predicado é satisfeito dado as path conditions.
///
/// Constrói no Z3: `(fact1 ∧ fact2 ∧ ... ∧ factN) ⟹ predicado`
/// e verifica se é tautologia (i.e., `facts ∧ ¬predicado` é UNSAT).
///
/// `pred_typed` é o predicado já tipado (Hole substituído pelo valor,
/// tipado via `infer_expr_hinted`). Os facts em `path_conditions` já são
/// `TypedExpr` (tipados no visitor de match/lambda).
///
/// Retorna:
/// - `Some(true)` — predicado provado satisfeito pelas path conditions.
/// - `Some(false)` — predicado refutado (path conditions implicam ¬predicado).
/// - `None` — sem path conditions, ou Z3 não decidiu (Unknown).
pub(crate) fn try_prove_with_path_conditions(
    pred_typed: &TypedExpr,
    path_conditions: &PathConditionCtx,
    inline_fns: &InlineFnTable,
) -> Option<bool> {
    if path_conditions.is_empty() {
        return None;
    }

    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();
        let mut translator = Z3PathTranslator::new(inline_fns);

        // Traduz facts como conjunção.
        let z3_facts: Vec<Bool> = path_conditions
            .facts()
            .iter()
            .map(|f| translator.translate_bool(f))
            .collect();
        let facts_conjunction = Bool::and(&z3_facts);

        // Traduz predicado.
        let z3_pred = translator.translate_bool(pred_typed);

        // Asserção: facts ∧ ¬predicado.
        // Se UNSAT, predicado é implicado pelas path conditions.
        solver.assert(facts_conjunction);
        solver.assert(z3_pred.not());

        match solver.check() {
            SatResult::Unsat => Some(true),
            SatResult::Sat => Some(false),
            SatResult::Unknown => None,
        }
    })
}

/// Tradutor de `TypedExpr` para expressões Z3.
///
/// Replica o `Z3Translator` de `guard_completeness.rs` — mesmas
/// traduções, mesmos fallbacks conservadores. Duplicado (não importado)
/// porque `Z3Translator` é privado ao módulo `guard_completeness`.
struct Z3PathTranslator<'a> {
    var_cache: std::collections::HashMap<String, VarKind>,
    fresh_counter: u32,
    inline_fns: &'a InlineFnTable,
}

enum VarKind {
    Int(z3::ast::Int),
    Bool(Bool),
}

impl<'a> Z3PathTranslator<'a> {
    fn new(inline_fns: &'a InlineFnTable) -> Self {
        Z3PathTranslator {
            var_cache: std::collections::HashMap::new(),
            fresh_counter: 0,
            inline_fns,
        }
    }

    fn fresh_bool(&mut self) -> Bool {
        let name = format!("__path_opaque_{}", self.fresh_counter);
        self.fresh_counter += 1;
        Bool::fresh_const(&name)
    }

    /// Tenta inlinar uma chamada de função pura. Se `name` está na
    /// `inline_fns` table, substitui os params pelos args e retorna o
    /// corpo tipado. O caller então traduz o corpo inlinado em vez da
    /// chamada. Retorna `None` se a função não está na tabela ou não
    /// tem corpo inlinable.
    fn try_inline(&self, name: &str, args: &[Spanned<TypedExpr>]) -> Option<TypedExpr> {
        let arg_types: Vec<Ty> = args.iter().map(|a| a.node.ty.clone()).collect();
        let fn_body = self.inline_fns.get(name, &arg_types)?;
        let body = fn_body.body.as_ref()?;
        let result = substitute_params(body, &fn_body.param_names, args);
        Some(result)
    }

    fn translate_bool(&mut self, expr: &TypedExpr) -> Bool {
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

    fn translate_comparison(&mut self, op: &str, args: &[Spanned<TypedExpr>]) -> Bool {
        if args.len() != 2 {
            return self.fresh_bool();
        }

        let lhs = self.translate_int(&args[0].node);
        let rhs = self.translate_int(&args[1].node);

        let (lhs, rhs) = match (lhs, rhs) {
            (Some(a), Some(b)) => (a, b),
            _ => return self.fresh_bool(),
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

    fn translate_int(&mut self, expr: &TypedExpr) -> Option<z3::ast::Int> {
        match &expr.kind {
            TypedExprKind::IntLit { text } => text.parse::<i64>().ok().map(z3::ast::Int::from_i64),
            TypedExprKind::Ident { name } => {
                if let Some(VarKind::Int(i)) = self.var_cache.get(name) {
                    Some(i.clone())
                } else {
                    let i = z3::ast::Int::new_const(name.as_str());
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
}
