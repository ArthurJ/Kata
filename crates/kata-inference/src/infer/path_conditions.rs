//! Path conditions — facts acumulados no contexto de inferência.
//!
//! Cada fact é um `TypedExpr` booleano conhecido como verdadeiro no ponto
//! atual do programa (ex: guard de match, condição de braço Boolean).
//! Quando uma ascription-refined é encontrada e `const_eval_predicate`
//! retorna `None` (não-literal), os facts são asserting no Z3 junto com
//! o predicado para provar implicação em compile-time.
//!
//! Nível 1 (PRD-refinement-propagation): guards locais apenas.

use std::collections::HashSet;

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
/// Mais uma loja de BINDINGS (não-facts):
/// - `let_bindings`: identidades `nome = valor` de bindings imutáveis
///   (`let`, params, sub-bindings de destructuring). Semeados no Z3
///   como aliasing (o tradutor memoiza o termo do VALOR sob o NOME) —
///   mais forte que asserir igualdade. Sound pelos mesmos motivos:
///   `let` é imutável e único por escopo (shadowing same-scope é erro;
///   sombreamento aninhado é last-wins com rollback do braço). `var`
///   nunca entra aqui (mutável, sem SSA).
///
/// O Z3 vê `facts + learned_facts` concatenados via `facts()`; os
/// bindings alimentam o tradutor ANTES da tradução (seeding).
/// `checkpoint()`/`rollback_to(n)` gerenciam o escopo: grava-se os
/// índices de `facts` e `let_bindings` antes do braço, truncam-se ao
/// sair (preservando `learned_facts` e bindings nascidos fora).
#[derive(Debug, Clone, Default)]
pub(crate) struct PathConditionCtx {
    facts: Vec<TypedExpr>,
    learned_facts: Vec<TypedExpr>,
    let_bindings: Vec<(String, TypedExpr)>,
    /// Nomes declarados `var` (mutáveis) em vigor. Alimenta o filtro
    /// conservador: facts que referenciam esses nomes são descartados
    /// na coleta (o valor pode mudar depois — fact stale é insound em
    /// ambas as direções: provar com ele aceita provas erradas, refutar
    /// com ele rejeita programas corretos).
    mutables: HashSet<String>,
}

impl PathConditionCtx {
    /// Registra um nome declarado `var` (mutável) no escopo atual.
    /// Alimenta o filtro da coleta: facts que referenciam `var` são
    /// descartados — o valor pode mudar após a coleta (reassign),
    /// tornando o fact stale. Conservador em ambas as direções: nunca
    /// provar nem refutar com material sobre mutável.
    pub(crate) fn add_mutable(&mut self, name: &str) {
        self.mutables.insert(name.to_string());
    }

    /// Adiciona um fact braço-específico (guard/Boolean).
    /// Rolled back ao sair do braço via `rollback_to`.
    ///
    /// **Filtro conservador (débito 1):** facts que referenciam
    /// bindings mutáveis (`var`) são descartados. O value de um var
    /// pode ser reatribuído depois da coleta; um fact stale prova
    /// porções erradas (P4) e refuta ascriptions corretas (P28).
    /// `dispatch` distingue free vars de nomes de função global
    /// (funções não são bindings mutáveis).
    pub(crate) fn add_fact(
        &mut self,
        fact: TypedExpr,
        dispatch: &kata_core::dispatch::DispatchTable,
    ) {
        if self.fact_references_mutable(&fact, dispatch) {
            return;
        }
        self.facts.push(fact);
    }

    /// True se algum Ident da expressão é um binding mutável registrado.
    ///
    /// Varredura completa (não só top-level): `match (> (* d 2) 10)`
    /// tem `d` aninhado em sub-expressões — o filtro precisa vê-lo.
    /// Não desce em Lambdas: captures de lambda que referenciam var
    /// são free vars do lambda, mas o corpo do lambda roda depois (o
    /// fact sobre a chamada não é sobre o valor atual do var).
    fn fact_references_mutable(
        &self,
        expr: &TypedExpr,
        dispatch: &kata_core::dispatch::DispatchTable,
    ) -> bool {
        let mut free = HashSet::new();
        let no_bindings = HashSet::new();
        super::free_vars::collect_free_vars(expr, &no_bindings, dispatch, &mut free);
        free.intersection(&self.mutables).next().is_some()
    }

    /// True se a expressão referencia algum binding mutável
    /// (`var`) — o Z3 deve ignorá-la: nem provar, nem refutar.
    /// Usado pelo gate de ascription (ascription sobre var é
    /// conservadoramente rejeitada, como quando não há facts).
    pub(crate) fn references_mutable(
        &self,
        expr: &TypedExpr,
        dispatch: &kata_core::dispatch::DispatchTable,
    ) -> bool {
        self.fact_references_mutable(expr, dispatch)
    }

    /// Adiciona un fact trans-escopo (Direção B — aprendido via dispatch).
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
    ///
    /// NOTA: bindings `let` NÃO entram no gate. O gate decide se há
    /// uma prova a tentar; bindings são definições, não restrições —
    /// sem facts a conjunção seria `true` e `true ⟹ ¬pred` refutaria
    /// qualquer predicado não-tautológico. O gate continua em facts.
    pub(crate) fn is_empty(&self) -> bool {
        self.facts.is_empty() && self.learned_facts.is_empty()
    }

    /// Registra um binding imutável (`let`, sub-binding de
    /// destructuring) para seeding no Z3.
    ///
    /// Append-only: sombreamento aninhado legal apenas empilha o novo
    /// binding — o last-wins é resolvido no seeding (a ordem de inserção
    /// no `var_cache` do tradutor dá last-wins naturalmente). Isto
    /// preserva a semântica de checkpoint/rollback: truncar remove os
    /// bindings do braço e o externo (empurrado antes do checkpoint)
    /// continua na loja. Remover o antigo aqui (retain) quebraria o
    /// rollback — a entrada externa teria sido apagada.
    pub(crate) fn add_let_binding(&mut self, name: &str, value: TypedExpr) {
        self.let_bindings.push((name.to_string(), value));
    }

    /// Bindings imutáveis acumulados, em ordem de registro. Nomes
    /// duplicados (sombreamento aninhado): o ÚLTIMO registro vence —
    /// resolvido no seeding, não aqui.
    pub(crate) fn let_bindings(&self) -> &[(String, TypedExpr)] {
        &self.let_bindings
    }

    /// Grava os índices atuais como checkpoint (antes de entrar num
    /// braço). `rollback_to` ao sair trunca `facts` e `let_bindings`
    /// de volta — bindings nascidos no braço morrem no braço; os de
    /// fora sobrevivem (trans-escopo, como learned_facts).
    pub(crate) fn checkpoint(&self) -> PathCheckPoint {
        PathCheckPoint {
            facts: self.facts.len(),
            lets: self.let_bindings.len(),
        }
    }

    /// Trunca `facts` e `let_bindings` de volta ao checkpoint,
    /// preservando `learned_facts` e bindings anteriores ao braço.
    pub(crate) fn rollback_to(&mut self, checkpoint: PathCheckPoint) {
        self.facts.truncate(checkpoint.facts);
        self.let_bindings.truncate(checkpoint.lets);
    }
}

/// Índices de escopo gravados por `PathConditionCtx::checkpoint`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PathCheckPoint {
    facts: usize,
    lets: usize,
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

        // Seeding: bindings `let` imutáveis do escopo em vigor viram
        // aliasing no var_cache — o predicado `> d 0` traduz pelo termo
        // do VALOR de `d` (ex: `> x 0`), conectando o binding aos
        // facts. Bindings não-traduzíveis viram variável livre
        // (fallback conservador). Sem isto, `d` é const livre e o Z3
        // não conecta `d = x`.
        translator.seed_let_bindings(path_conditions.let_bindings());

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
