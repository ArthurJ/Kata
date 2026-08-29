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

use z3::{Config, SatResult, Solver, ast::Bool, with_z3_config};

use super::post_conditions::InlineFnTable;
use crate::z3_translate::Z3Translator;

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
        let mut translator = Z3Translator::with_inline_fns(inline_fns);

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
