//! Testes de completude de guards — verifica se a disjunção das
//! condições dos guards é uma tautologia usando Z3.
//!
//! Cobre os casos do PRD §6.2:
//! - Guards com otherwise → Ok
//! - Guards complementares sem otherwise → Ok (Z3 prova tautologia)
//! - Guards não-exaustivos sem otherwise → NonExhaustiveMatch
//! - Guard único sem condição → Ok
//! - Sem guards → Ok

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

// ── Helpers (duplicados de lambda_match_inference.rs para isolamento) ──

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

// ── Testes ───────────────────────────────────────────────────────

/// Guards com otherwise → trivialmente exaustivo. Sem Z3.
#[test]
fn guard_with_otherwise_ok() {
    let src = "\
abs :: Int => Int\n\
lambda x:\n\
\x20   > x 0: x\n\
\x20   otherwise: - 0 x\n\
abs 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Guards complementares sem otherwise: `> x 0` e `<= x 0`.
/// Disjunção é tautologia — Z3 prova UNSAT.
#[test]
fn guard_complementary_without_otherwise_ok() {
    let src = "\
foo :: Int => Int\n\
lambda x:\n\
\x20   > x 0: x\n\
\x20   <= x 0: - 0 x\n\
foo 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Guards não-exaustivos sem otherwise: só `> x 0`.
/// Z3 encontra contra-exemplo (x = -1 ou x = 0).
#[test]
fn guard_non_exhaustive_without_otherwise_error() {
    let src = "\
foo :: Int => Int\n\
lambda x:\n\
\x20   > x 0: x\n\
foo 5";
    let err = infer_src_err(src);
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
}

/// Guard único sem condição (`otherwise: x`) → Ok.
#[test]
fn guard_single_otherwise_ok() {
    let src = "\
foo :: Int => Int\n\
lambda x:\n\
\x20   otherwise: x\n\
foo 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Sem guards (body direto) → Ok, não se aplica.
#[test]
fn no_guards_ok() {
    let src = "\
foo :: Int => Int\n\
lambda x: x\n\
foo 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Guards com otherwise cobrindo caso opaco → Ok.
#[test]
fn guard_with_otherwise_and_opaque_ok() {
    let src = "\
foo :: Int => Int\n\
lambda x:\n\
\x20   > x 0: x\n\
\x20   otherwise: 0\n\
foo 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Guards com 3 condições complementares sem otherwise:
/// `> x 0`, `= x 0`, `< x 0`. Disjunção é tautologia.
#[test]
fn guard_three_complementary_without_otherwise_ok() {
    let src = "\
sign :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
\x20   = x 0: 0\n\
\x20   < x 0: - 0 1\n\
sign 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Guards com `and` na condição: `and (> x 0) (< x 10)` sem otherwise.
/// Não cobre x <= 0 ou x >= 10 → NonExhaustiveMatch.
#[test]
fn guard_with_and_non_exhaustive_error() {
    let src = "\
foo :: Int => Int\n\
lambda x:\n\
\x20   and (> x 0) (< x 10): x\n\
foo 5";
    let err = infer_src_err(src);
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
}

/// Guards com `and` e `not` complementares: `and (> x 0) (< x 10)`
/// e `not (and (> x 0) (< x 10))` → tautologia.
#[test]
fn guard_with_and_not_complementary_ok() {
    let src = "\
foo :: Int => Int\n\
lambda x:\n\
\x20   and (> x 0) (< x 10): x\n\
\x20   not (and (> x 0) (< x 10)): 0\n\
foo 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}
