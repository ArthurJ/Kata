//! Completude de guards — verifica se a disjunção das condições dos
//! guards é uma tautologia usando Z3 (SMT solver).
//!
//! Fluxo:
//! 1. Se algum guard tem `condition: None` (`otherwise`) → trivialmente
//!    exaustivo, Ok sem Z3.
//! 2. Senão, traduzir cada condição `TypedExpr` → expressão Z3.
//! 3. Construir a disjunção de todas as condições.
//! 4. Asserção: negação da disjunção.
//! 5. `solver.check()`:
//!    - `Unsat` → tautologia provada, Ok.
//!    - `Sat` → contra-exemplo encontrado, `NonExhaustiveMatch`.
//!    - `Unknown` → limite atingido, `MissingOtherwise`.

use std::collections::HashMap;

use kata_ast::Span;
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind, TypedGuardClause};

use z3::{
    Config, SatResult, Solver,
    ast::{Bool, Int},
    with_z3_config,
};

/// Resultado da verificação de completude de guards.
pub(crate) type GuardResult = Result<(), MiddleError>;

/// Verifica se os guards são exaustivos.
///
/// Se algum guard tem `condition: None` (`otherwise`), trivialmente Ok.
/// Senão, usa Z3 para provar que a disjunção das condições é tautologia.
pub(crate) fn check_guard_completeness(guards: &[TypedGuardClause], span: &Span) -> GuardResult {
    // otherwise presente — trivialmente exaustivo.
    if guards.iter().any(|g| g.condition.is_none()) {
        return Ok(());
    }

    // Coleta as condições (todas são Some porque verificamos otherwise acima).
    let conditions: Vec<TypedExpr> = guards
        .iter()
        .filter_map(|g| g.condition.as_ref().map(|c| c.node.clone()))
        .collect();

    if conditions.is_empty() {
        // Sem guards e sem otherwise — nada a verificar.
        return Ok(());
    }

    // Configura Z3 com rlimit para determinismo de esforço.
    let mut cfg = Config::new();
    cfg.set_param_value("rlimit", "10000");

    let span_val = *span;

    with_z3_config(&cfg, || {
        let solver = Solver::new();

        // Traduz cada condição para Z3 Bool.
        let mut translator = Z3Translator::new();
        let z3_conditions: Vec<Bool> = conditions
            .iter()
            .map(|cond| translator.translate_bool(cond))
            .collect();

        // Constrói a disjunção de todas as condições.
        let disjunction = Bool::or(&z3_conditions);

        // Para provar que a disjunção é tautologia, verificamos se
        // a NEGAÇÃO da disjunção é insatisfazível.
        solver.assert(disjunction.not());

        match solver.check() {
            SatResult::Unsat => {
                // ¬(cond1 ∨ ... ∨ condN) é insatisfazível → disjunção é tautologia.
                GuardResult::Ok(())
            }
            SatResult::Sat => {
                // ¬(cond1 ∨ ... ∨ condN) é satisfazível → existe contra-exemplo.
                let model = solver.get_model().unwrap();
                let counter_example = translator.extract_counter_example(&model);
                Err(MiddleError::NonExhaustiveMatch {
                    missing: vec![counter_example],
                    span: span_val.into(),
                    hint: Some(
                        "guards não cobrem todos os casos. \
                         Adicione um guard ou use `otherwise:` como fallback"
                            .to_string(),
                    ),
                })
            }
            SatResult::Unknown => {
                // Z3 não decidiu a tempo — exigir otherwise.
                Err(MiddleError::MissingOtherwise {
                    span: span_val.into(),
                })
            }
        }
    })
}

/// Tradutor de `TypedExpr` para expressões Z3.
///
/// Mapeia operações Kata5 para as teorias correspondentes do Z3:
/// - Literais Int → `Int::from_i64`
/// - `> a b`, `< a b`, `>= a b`, `<= a b` → operações Z3
/// - `= a b`, `!= a b` → `=` Z3
/// - `+ a b`, `- a b`, `* a b` → aritmética Z3
/// - `and a b`, `or a b`, `not a` → lógica proposicional Z3
/// - Variáveis (`x`) → `Int::new_const` (assume Int por padrão)
/// - Qualquer outra → variável booleana opaca
struct Z3Translator {
    /// Nomes de variáveis já criadas, para reutilizar.
    var_cache: HashMap<String, VarKind>,
    /// Contador para variáveis opacas frescas.
    fresh_counter: u32,
}

enum VarKind {
    Int(Int),
    Bool(Bool),
}

impl Z3Translator {
    fn new() -> Self {
        Z3Translator {
            var_cache: HashMap::new(),
            fresh_counter: 0,
        }
    }

    fn fresh_bool(&mut self) -> Bool {
        let name = format!("__opaque_{}", self.fresh_counter);
        self.fresh_counter += 1;
        Bool::fresh_const(&name)
    }

    /// Traduz uma expressão para um Z3 Bool.
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
                        _ => self.fresh_bool(),
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

    /// Traduz uma comparação (`>`, `<`, `>=`, `<=`, `=`, `!=`).
    fn translate_comparison(&mut self, op: &str, args: &[kata_ast::Spanned<TypedExpr>]) -> Bool {
        if args.len() != 2 {
            return self.fresh_bool();
        }

        // Tenta traduzir ambos os operandos como Int.
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
                        _ => None,
                    }
                } else {
                    None
                }
            }
            TypedExprKind::Grouping { inner } => self.translate_int(&inner.node),
            _ => None,
        }
    }

    /// Extrai o contra-exemplo do modelo Z3.
    fn extract_counter_example(&self, model: &z3::Model) -> String {
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
            })
            .collect();

        if parts.is_empty() {
            "caso não coberto pelos guards".to_string()
        } else {
            parts.join(", ")
        }
    }
}
