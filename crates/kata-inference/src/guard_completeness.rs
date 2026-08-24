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
//!
//! Também oferece `check_guard_implication` para verificação de
//! redundância de cláusulas: prova se `guards_N ⟹ guards_M`.

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

/// Resultado trivalorado de uma prova Z3.
///
/// `Proven` = UNSAT (propriedade provada).
/// `Refuted` = SAT (contra-exemplo existe).
/// `Unknown` = Z3 não decidiu (limite de esforço).
enum Ternary {
    Proven,
    Refuted,
    Unknown,
}

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

    let span_val = *span;

    match prove_tautology(&conditions) {
        Ternary::Proven => Ok(()),
        Ternary::Refuted => {
            // ¬(cond1 ∨ ... ∨ condN) é satisfazível → existe contra-exemplo.
            // Precisa reexecutar com modelo para extrair o contra-exemplo.
            let counter_example = prove_tautology_with_model(&conditions);
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
        Ternary::Unknown => {
            // Z3 não decidiu a tempo — exigir otherwise.
            Err(MiddleError::MissingOtherwise {
                span: span_val.into(),
            })
        }
    }
}

/// Verifica se os guards de N implicam os guards de M.
///
/// Prova: `guards_N ⟹ guards_M`, i.e., `guards_N ∧ ¬guards_M` é UNSAT.
///
/// Retorna `true` se a implicação foi provada (N é redundante).
/// Retorna `false` se refutada (SAT — contra-exemplo existe) ou se
/// Z3 não decidiu (Unknown — conservador, assume não-redundante).
pub(crate) fn check_guard_implication(
    guards_n: &[TypedGuardClause],
    guards_m: &[TypedGuardClause],
    _span: &Span,
) -> bool {
    // Se N tem otherwise (condition: None), disj_N é True.
    // True ∧ ¬disj_M = ¬disj_M. Se disj_M é tautologia, UNSAT → true.
    // Se disj_M não é tautologia, SAT → false. Correto.
    //
    // Se M tem otherwise, disj_M é True. ¬True = False.
    // disj_N ∧ False = False → UNSAT → true.
    // Correto: M com otherwise sempre dispara, N é redundante.

    let conditions_n: Vec<TypedExpr> = guards_n
        .iter()
        .filter_map(|g| g.condition.as_ref().map(|c| c.node.clone()))
        .collect();
    let conditions_m: Vec<TypedExpr> = guards_m
        .iter()
        .filter_map(|g| g.condition.as_ref().map(|c| c.node.clone()))
        .collect();

    // Se N não tem condições (nem otherwise), não há como N disparar —
    // não é redundante por implicação de guards (pode ser por pattern alone).
    if conditions_n.is_empty() && !guards_n.iter().any(|g| g.condition.is_none()) {
        return false;
    }

    match prove_implication(&conditions_n, &conditions_m, guards_n, guards_m) {
        Ternary::Proven => true,
        Ternary::Refuted | Ternary::Unknown => false,
    }
}

// ── Funções internas de prova Z3 ─────────────────────────────────────

/// Configura Z3 com rlimit para determinismo de esforço.
fn z3_config() -> Config {
    let mut cfg = Config::new();
    cfg.set_param_value("rlimit", "10000");
    cfg
}

/// Prova se a disjunção de `conditions` é tautologia.
///
/// Verifica se `¬(cond1 ∨ ... ∨ condN)` é insatisfazível.
fn prove_tautology(conditions: &[TypedExpr]) -> Ternary {
    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();
        let mut translator = Z3Translator::new();

        let z3_conditions: Vec<Bool> = conditions
            .iter()
            .map(|cond| translator.translate_bool(cond))
            .collect();

        let disjunction = Bool::or(&z3_conditions);
        solver.assert(disjunction.not());

        match solver.check() {
            SatResult::Unsat => Ternary::Proven,
            SatResult::Sat => Ternary::Refuted,
            SatResult::Unknown => Ternary::Unknown,
        }
    })
}

/// Reexecuta a prova de tautologia extraindo o contra-exemplo do modelo.
///
/// Usado quando `prove_tautology` retorna `Refuted` e precisamos do
/// contra-exemplo para a mensagem de erro.
fn prove_tautology_with_model(conditions: &[TypedExpr]) -> String {
    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();
        let mut translator = Z3Translator::new();

        let z3_conditions: Vec<Bool> = conditions
            .iter()
            .map(|cond| translator.translate_bool(cond))
            .collect();

        let disjunction = Bool::or(&z3_conditions);
        solver.assert(disjunction.not());

        if let SatResult::Sat = solver.check()
            && let Some(model) = solver.get_model()
        {
            return translator.extract_counter_example(&model);
        }

        "caso não coberto pelos guards".to_string()
    })
}

/// Prova se `conditions_n ⟹ conditions_m`.
///
/// Constrói `disj_n ∧ ¬disj_m` e verifica satisfatibilidade.
/// Se UNSAT, a implicação é provada.
///
/// `guards_n`/`guards_m` são passados para detectar `otherwise`
/// (condition: None), que faz a disjunção ser trivialmente `True`.
fn prove_implication(
    conditions_n: &[TypedExpr],
    conditions_m: &[TypedExpr],
    guards_n: &[TypedGuardClause],
    guards_m: &[TypedGuardClause],
) -> Ternary {
    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();
        let mut translator = Z3Translator::new();

        // disj_N: se N tem otherwise, é True. Senão, disjunção das condições.
        let n_has_otherwise = guards_n.iter().any(|g| g.condition.is_none());
        let disj_n = if n_has_otherwise {
            Bool::from_bool(true)
        } else if conditions_n.is_empty() {
            // Sem condições e sem otherwise — disjunção vazia = False.
            Bool::from_bool(false)
        } else {
            let z3_conds: Vec<Bool> = conditions_n
                .iter()
                .map(|c| translator.translate_bool(c))
                .collect();
            Bool::or(&z3_conds)
        };

        // disj_M: se M tem otherwise, é True. Senão, disjunção das condições.
        let m_has_otherwise = guards_m.iter().any(|g| g.condition.is_none());
        let disj_m = if m_has_otherwise {
            Bool::from_bool(true)
        } else if conditions_m.is_empty() {
            Bool::from_bool(false)
        } else {
            let z3_conds: Vec<Bool> = conditions_m
                .iter()
                .map(|c| translator.translate_bool(c))
                .collect();
            Bool::or(&z3_conds)
        };

        // Asserção: disj_N ∧ ¬disj_M
        solver.assert(disj_n);
        solver.assert(disj_m.not());

        match solver.check() {
            SatResult::Unsat => Ternary::Proven,
            SatResult::Sat => Ternary::Refuted,
            SatResult::Unknown => Ternary::Unknown,
        }
    })
}

// ── Tradutor TypedExpr → Z3 ──────────────────────────────────────────

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
