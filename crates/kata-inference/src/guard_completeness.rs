//! Completude de guards — verifica se a disjunção das condições dos
//! guards é uma tautologia usando Z3 (SMT solver).
//!
//! Fluxo:
//! 1. Se algum guard tem `condition: None` (`otherwise`) → trivialmente
//!    exaustivo, Ok sem Z3.
//! 2. Senão, traduzir cada condição `TypedExpr` → expressão Z3.
//!    ANTES disso, semear os bindings do `with` da cláusula no tradutor
//!    (`seed_with_bindings`) — sem isso, um guard `neg:` cujo valor vem de
//!    `neg := < x 0` vira `Ident("neg")` → Bool livre no solver, e a
//!    prova falha sempre (bug 2026-08-29).
//! 3. Construir a disjunção de todas as condições.
//! 4. Asserção: negação da disjunção.
//! 5. `solver.check()`:
//!    - `Unsat` → tautologia provada, Ok.
//!    - `Sat` → contra-exemplo encontrado, `NonExhaustiveMatch`.
//!    - `Unknown` → limite atingido, `MissingOtherwise`.
//!
//! Também oferece `check_guard_implication` para verificação de
//! redundância de cláusulas: prova se `guards_N ⟹ guards_M`.

use kata_ast::Span;
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedGuardClause};
use crate::typed_pattern::TypedWithBinding;
use crate::z3_translate::Z3Translator;

use z3::{Config, SatResult, Solver, ast::Bool, with_z3_config};

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
///
/// `with_bindings` são os bindings `with` da cláusula (`nome := expr`).
/// São traduzidos primeiro e memoizados: guards que referenciam um
/// binding (`neg: ...` com `neg := < x 0`) provam sobre o valor REAL,
/// não sobre um Bool livre.
pub(crate) fn check_guard_completeness(
    guards: &[TypedGuardClause],
    with_bindings: &[TypedWithBinding],
    span: &Span,
) -> GuardResult {
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

    match prove_tautology(&conditions, with_bindings) {
        Ternary::Proven => Ok(()),
        Ternary::Refuted => {
            // ¬(cond1 ∨ ... ∨ condN) é satisfazível → existe contra-exemplo.
            // Precisa reexecutar com modelo para extrair o contra-exemplo.
            let counter_example = prove_tautology_with_model(&conditions, with_bindings);
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
/// Cada cláusula tem seus PRÓPRIOS bindings `with` — `with_n` são os
/// bindings da cláusula N, `with_m` os da cláusula M. A prova usa um
/// tradutor por cláusula? NÃO — a implicação é sobre o MESMO input:
/// bindings de N e M são avaliados sobre os mesmos params, então um
/// único tradutor semeados com `with_n` + `with_m` seria INCORRETO se
/// os nomes colidissem com semânticas diferentes.
///
/// Hmm — na verdade, cada cláusula de uma função multi-cláusula recebe
/// os mesmos argumentos. Bindings do `with` são computações puras sobre
/// os params; dois bindings com o mesmo nome em cláusulas diferentes
/// computam o mesmo valor se as expressões forem iguais, mas podem ser
/// expressões diferentes. O correto é não compartilhar: a implicação
/// `guards_N ⟹ guards_M` quantifica sobre inputs onde AMBAS as
/// cláusulas casam os patterns — e cada guarda é avaliada com seus
/// próprios bindings.
///
/// Simplificação adotada (documentada): um único tradutor, semeado com
/// os bindings de N seguidos dos bindings de M (M sobrescreve N em
/// colisão de nome). Se N e M definem o mesmo nome com expressões
/// diferentes, o valor de M vence — conservador? NÃO, é INCORRETO...
///
/// DECISÃO: dois tradutores separados, um por cláusula, e a prova
/// conjuga as traduções. Como as condições de N referenciam bindings
/// de N (e M os de M), traduzir cada conjunto com seu próprio tradutor
/// e combinar as Bools resultantes é a única forma correta — as vars
/// de params (x) são compartilhadas por NOME entre tradutores (mesma
/// const Z3 "x"), e bindings de nomes distintos não colidem.
///
/// Retorna `true` se a implicação foi provada (N é redundante).
/// Retorna `false` se refutada (SAT — contra-exemplo existe) ou se
/// Z3 não decidiu (Unknown — conservador, assume não-redundante).
pub(crate) fn check_guard_implication(
    guards_n: &[TypedGuardClause],
    guards_m: &[TypedGuardClause],
    with_n: &[TypedWithBinding],
    with_m: &[TypedWithBinding],
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

    match prove_implication(
        &conditions_n,
        &conditions_m,
        guards_n.iter().any(|g| g.condition.is_none()),
        guards_m.iter().any(|g| g.condition.is_none()),
        with_n,
        with_m,
    ) {
        Ternary::Proven => true,
        Ternary::Refuted | Ternary::Unknown => false,
    }
}

// ── Verificação de cobertura de guards entre múltiplas cláusulas ────

/// Braço com guards para verificação de cobertura entre cláusulas.
/// Cada braço contribui com seus guards e seus bindings `with`.
pub(crate) struct GuardArm<'a> {
    pub guards: &'a [TypedGuardClause],
    pub with_bindings: &'a [TypedWithBinding],
}

/// Verifica se a disjunção dos guards de múltiplos braços cobre todos os casos.
///
/// Usada pela Fase 3: quando o motor Maranget confirma cobertura estrutural,
/// esta função prova que os guards de todos os braços que casam uma folha
/// específica formam uma tautologia disjuntiva.
///
/// Se algum braço tem `otherwise` (`condition: None`), a disjunção é
/// trivialmente `True` — Ok sem Z3.
///
/// Cada braço é traduzido por seu PRÓPRIO tradutor Z3 (semearado com seus
/// `with_bindings`), evitando colisão de nomes entre bindings de cláusulas
/// diferentes. Params (ex: `x`) são consts Z3 por nome — compartilhados
/// naturalmente entre tradutores, o que é correto: a prova quantifica sobre
/// o mesmo input.
pub(crate) fn check_guard_coverage(arms: &[GuardArm], span: &Span) -> GuardResult {
    // Se algum braço tem otherwise, a disjunção é trivialmente True.
    if arms
        .iter()
        .any(|a| a.guards.iter().any(|g| g.condition.is_none()))
    {
        return Ok(());
    }

    // Coleta todas as condições de todos os braços.
    let all_conditions: Vec<TypedExpr> = arms
        .iter()
        .flat_map(|a| {
            a.guards
                .iter()
                .filter_map(|g| g.condition.as_ref().map(|c| c.node.clone()))
        })
        .collect();

    if all_conditions.is_empty() {
        // Nenhum braço tem guards nem otherwise — nada a verificar.
        // Isto não deveria acontecer se a função foi chamada corretamente
        // (só é chamada quando há braços com guards), mas é defensivo.
        return Ok(());
    }

    let span_val = *span;

    match prove_guard_coverage(arms) {
        Ternary::Proven => Ok(()),
        Ternary::Refuted => {
            let counter_example = prove_guard_coverage_with_model(arms);
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
        Ternary::Unknown => Err(MiddleError::MissingOtherwise {
            span: span_val.into(),
        }),
    }
}

/// Prova se a disjunção dos guards de múltiplos braços é tautologia.
///
/// `¬(g₁ ∨ … ∨ gₙ)` UNSAT → tautologia provada.
/// Cada braço traduz seus guards com seu próprio tradutor Z3.
fn prove_guard_coverage(arms: &[GuardArm]) -> Ternary {
    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();

        let mut all_z3_conds: Vec<Bool> = Vec::new();

        for arm in arms {
            let conditions: Vec<TypedExpr> = arm
                .guards
                .iter()
                .filter_map(|g| g.condition.as_ref().map(|c| c.node.clone()))
                .collect();

            if conditions.is_empty() {
                continue;
            }

            let mut translator = Z3Translator::new();
            translator.seed_with_bindings(arm.with_bindings);

            let z3_conds: Vec<Bool> = conditions
                .iter()
                .map(|cond| translator.translate_bool(cond))
                .collect();

            all_z3_conds.extend(z3_conds);
        }

        if all_z3_conds.is_empty() {
            // Sem condições — disjunção vazia = False, ¬False = True (SAT).
            // Não é tautologia. Mas este caso é tratado por check_guard_coverage
            // (otherwise ou vazio → Ok antes de chegar aqui).
            return Ternary::Refuted;
        }

        let disjunction = Bool::or(&all_z3_conds);
        solver.assert(disjunction.not());

        match solver.check() {
            SatResult::Unsat => Ternary::Proven,
            SatResult::Sat => Ternary::Refuted,
            SatResult::Unknown => Ternary::Unknown,
        }
    })
}

/// Reexecuta a prova de cobertura extraindo o contra-exemplo do modelo.
fn prove_guard_coverage_with_model(arms: &[GuardArm]) -> String {
    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();

        let mut all_z3_conds: Vec<Bool> = Vec::new();
        // Usa o último tradutor para extrair o contra-exemplo — params são
        // compartilhados por nome entre tradutores, então qualquer um serve.
        let mut last_translator = Z3Translator::new();

        for arm in arms {
            let conditions: Vec<TypedExpr> = arm
                .guards
                .iter()
                .filter_map(|g| g.condition.as_ref().map(|c| c.node.clone()))
                .collect();

            if conditions.is_empty() {
                continue;
            }

            let mut translator = Z3Translator::new();
            translator.seed_with_bindings(arm.with_bindings);

            let z3_conds: Vec<Bool> = conditions
                .iter()
                .map(|cond| translator.translate_bool(cond))
                .collect();

            all_z3_conds.extend(z3_conds);
            last_translator = translator;
        }

        if all_z3_conds.is_empty() {
            return "caso não coberto pelos guards".to_string();
        }

        let disjunction = Bool::or(&all_z3_conds);
        solver.assert(disjunction.not());

        if let SatResult::Sat = solver.check()
            && let Some(model) = solver.get_model()
        {
            return last_translator.extract_counter_example(&model);
        }

        "caso não coberto pelos guards".to_string()
    })
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
fn prove_tautology(conditions: &[TypedExpr], with_bindings: &[TypedWithBinding]) -> Ternary {
    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();
        let mut translator = Z3Translator::new();

        // Semear bindings do with ANTES das condições: guards via `with`
        // referenciam os bindings, e a referência deve provar sobre o
        // valor real (memoizado), não sobre um Bool livre.
        translator.seed_with_bindings(with_bindings);

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
fn prove_tautology_with_model(
    conditions: &[TypedExpr],
    with_bindings: &[TypedWithBinding],
) -> String {
    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();
        let mut translator = Z3Translator::new();
        translator.seed_with_bindings(with_bindings);

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
/// Tradução por cláusula: N com um tradutor semeado com `with_n`, M com
/// um tradutor semeado com `with_m`. Params (ex: `x`) são consts Z3 por
/// nome — compartilhados naturalmente entre os dois tradutores, o que
/// é correto: a implicação quantifica sobre o mesmo input. Bindings de
/// `with` com o mesmo nome em N e M mas expressões diferentes recebem
/// vars Z3 distintas por tradutor (sem vazamento entre cláusulas).
///
/// `otherwise` em N/M é detectado em `check_guard_implication` e passado
/// aqui como `n_has_otherwise`/`m_has_otherwise` — otherwise faz a
/// disjunção ser trivialmente `True`.
fn prove_implication(
    conditions_n: &[TypedExpr],
    conditions_m: &[TypedExpr],
    n_has_otherwise: bool,
    m_has_otherwise: bool,
    with_n: &[TypedWithBinding],
    with_m: &[TypedWithBinding],
) -> Ternary {
    let cfg = z3_config();

    with_z3_config(&cfg, || {
        let solver = Solver::new();
        let mut translator_n = Z3Translator::new();
        translator_n.seed_with_bindings(with_n);
        let mut translator_m = Z3Translator::new();
        translator_m.seed_with_bindings(with_m);

        // disj_N: se N tem otherwise, é True. Senão, disjunção das
        // condições (tradutor próprio de N).
        let disj_n = if n_has_otherwise {
            Bool::from_bool(true)
        } else if conditions_n.is_empty() {
            // Sem condições e sem otherwise — disjunção vazia = False.
            Bool::from_bool(false)
        } else {
            let z3_conds: Vec<Bool> = conditions_n
                .iter()
                .map(|c| translator_n.translate_bool(c))
                .collect();
            Bool::or(&z3_conds)
        };

        // disj_M: se M tem otherwise, é True. Senão, disjunção das
        // condições (tradutor próprio de M).
        let disj_m = if m_has_otherwise {
            Bool::from_bool(true)
        } else if conditions_m.is_empty() {
            // Sem condições e sem otherwise — disjunção vazia = False.
            Bool::from_bool(false)
        } else {
            let z3_conds: Vec<Bool> = conditions_m
                .iter()
                .map(|c| translator_m.translate_bool(c))
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
