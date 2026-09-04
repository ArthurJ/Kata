//! Testes de cláusulas redundantes (DoD 12) e redundância com guards.
//!
//! Fase 1: tautologia dos guards de M.
//! Fase 2: implicação guards_N ⟹ guards_M.

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

// ── Helpers (duplicados de infer_test.rs para isolamento) ─────────

/// Combina prelude + módulo do usuário (replica do named_functions_e2e.rs).
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: {
            let mut ir = prelude.interface_registry.clone();
            ir.merge(user.interface_registry.clone());
            ir
        },
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
        },
        type_graph: {
            let mut tg = prelude.type_graph.clone();
            tg.merge(&user.type_graph);
            tg
        },
        functions: {
            let mut fns = prelude.functions;
            let user_fn_names: std::collections::HashSet<&str> =
                user.functions.iter().map(|f| f.name.as_str()).collect();
            fns.retain(|f| !user_fn_names.contains(f.name.as_str()));
            fns.extend(user.functions);
            fns
        },
        actions: {
            let mut acts = prelude.actions;
            let user_action_names: std::collections::HashSet<&str> =
                user.actions.iter().map(|a| a.name.as_str()).collect();
            acts.retain(|a| !user_action_names.contains(a.name.as_str()));
            acts.extend(user.actions);
            acts
        },
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── DoD 12: RedundantClause — cláusulas sobrepostas ──────────────

/// Cláusula wildcard seguida de cláusula ident → redundante.
/// `lambda _: 1` cobre tudo; `lambda n: n` é inalcançável.
#[test]
fn redundant_clause_wildcard_then_ident() {
    let src = "\
fun :: Int => Int\n\
lambda _: 1\n\
lambda n: n\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause, got {err:?}"
    );
}

/// Cláusula ident seguida de cláusula ident → redundante.
/// `lambda x: 1` cobre tudo; `lambda y: 2` é inalcançável.
#[test]
fn redundant_clause_ident_then_ident() {
    let src = "\
fun :: Int => Int\n\
lambda x: 1\n\
lambda y: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause, got {err:?}"
    );
}

/// Cláusula literal seguida da mesma literal → redundante.
/// `lambda 0: 1` cobre 0; `lambda 0: 2` é inalcançável.
#[test]
fn redundant_clause_same_literal() {
    let src = "\
fun :: Int => Int\n\
lambda 0: 1\n\
lambda 0: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause, got {err:?}"
    );
}

/// Cláusulas não-sobrepostas NÃO produzem RedundantClause.
/// `lambda 0: 1` e `lambda n: n` não são sobrepostas.
#[test]
fn non_redundant_clauses_ok() {
    let src = "\
fun :: Int => Int\n\
lambda 0: 1\n\
lambda n: n\n\
fun 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Cláusula sem guards seguida de cláusula com guards E mesmos patterns
/// → redundante. M sem guards sempre dispara sobre os patterns,capturando
/// o input antes de N ser avaliada.
/// `lambda x: 1` (sem guards, Ident cobre tudo) → `lambda x: guards` é redundante.
#[test]
fn redundant_clause_no_guards_covers_guarded() {
    let src = "\
fun :: Int => Int\n\
lambda x: 1\n\
lambda x:\n\
\x20   > x 0: 2\n\
\x20   otherwise: 3\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause, got {err:?}"
    );
}

// ── Redundância com guards: Fase 1 (tautologia dos guards de M) ─────

/// M com guards tautológicos (x > 0 ∨ x <= 0 = True) cobre tudo.
/// N sem guards com mesmo pattern é redundante.
/// Não precisa de otherwise: os guards são tautológicos (Z3 prova).
#[test]
fn redundant_clause_guarded_m_tautology_n_no_guards() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
\x20   <= x 0: 2\n\
lambda x: 3\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause (guards de M são tautologia), got {err:?}"
    );
}

/// M com otherwise (trivialmente tautologia) cobre tudo.
/// N sem guards com mesmo pattern é redundante.
#[test]
fn redundant_clause_guarded_m_otherwise_n_no_guards() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
\x20   otherwise: 2\n\
lambda x: 3\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause (M tem otherwise), got {err:?}"
    );
}

/// M com guards NÃO-tautológicos não pode ser testado isoladamente:
/// se M tem guards sem otherwise e não-tautologia, `check_guard_completeness`
/// dispara NonExhaustiveMatch durante a inferência (antes de
/// `check_redundant_clauses`). O caso (true, false) onde M tem guards
/// não-tautológicos simplesmente nunca chega à verificação de redundância.
///
/// O teste abaixo usa M com guards tautológicos (otherwise) + N sem guards.
/// N é redundante porque M sempre dispara.
/// Para testar não-redundância com guards, ver Fase 2 (guard_implication).
//
// ── Redundância com guards: Fase 2 (implicação guards_N ⟹ guards_M) ─
/// Guards de N implicam guards de M: x > 5 ⟹ x > 0.
/// N é redundante — M dispara antes para todo input que N casaria.
/// Não precisam de otherwise: a redundância roda antes da exaustividade
/// de guards.
#[test]
fn redundant_clause_guard_implication() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
lambda x:\n\
\x20   > x 5: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause (x > 5 implica x > 0), got {err:?}"
    );
}

/// Guards de N NÃO implicam guards de M: x <= 5 não implica x > 0.
/// N não é redundante — x = -1 satisfaz N mas não M.
///
/// Pós-Fase 3: os guards `> x 0` e `<= x 5` juntos cobrem todo Int
/// (tautologia disjuntiva provada por Z3), então a função é exaustiva
/// e não há erro de exaustividade nem RedundantClause.
#[test]
fn non_redundant_guard_no_implication() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
lambda x:\n\
\x20   <= x 5: 2\n\
fun 5";
    // Pós-Fase 3: a função é exaustiva (guards disjuntivos cobrem Int).
    // Não deve haver erro de inferência.
    let _module = infer_src(src);
}

/// Guards idênticos: x > 0 ⟹ x > 0 (trivialmente verdadeiro).
/// N é redundante — M dispara primeiro com o mesmo guard.
#[test]
fn redundant_clause_identical_guards() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
lambda x:\n\
\x20   > x 0: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause (guards idênticos), got {err:?}"
    );
}

/// Guards disjuntos: x < 0 não implica x > 10.
/// N não é redundante. O erro deve ser NonExhaustiveMatch, não
/// RedundantClause.
#[test]
fn non_redundant_disjoint_guards() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 10: 1\n\
lambda x:\n\
\x20   < x 0: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        !matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "não esperava RedundantClause, got {err:?}"
    );
}

/// Multi-cláusula com variantes de enum Boolean NÃO é redundante.
/// `lambda True True: True` seguido de `lambda True False: False` —
/// variantes diferentes não se cobrem. Antes do fix, o checker operava
/// sobre `Pattern::Ident("True")` (pré-typeck) e tratava todo `Ident`
/// como wildcard, causando falso positivo.
#[test]
fn non_redundant_boolean_variant_clauses() {
    let src = "\
and :: Boolean Boolean => Boolean\n\
lambda True True: True\n\
lambda True False: False\n\
lambda False True: False\n\
lambda False False: False\n\
and True False";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}
