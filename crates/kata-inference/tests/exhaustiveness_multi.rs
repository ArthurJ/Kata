//! Testes de exaustividade com N parâmetros (produto cartesiano).

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

// ── Exaustividade de N parâmetros (produto cartesiano) ───────────

/// 2 Boolean exaustivo: 4 cláusulas cobrindo True×True, True×False,
/// False×True, False×False.
#[test]
fn exhaustiveness_2_boolean_exhaustive() {
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

/// 2 Boolean não-exaustivo: 2 cláusulas (True×True, True×False).
/// Faltam (False, True) e (False, False) → NonExhaustiveMatch.
#[test]
fn exhaustiveness_2_boolean_non_exhaustive() {
    let src = "\
and :: Boolean Boolean => Boolean\n\
lambda True True: True\n\
lambda True False: False\n\
and True False";
    let err = infer_src_err(src);
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
    if let kata_diagnostics::MiddleError::NonExhaustiveMatch { missing, .. } = err {
        assert_eq!(
            missing.len(),
            2,
            "deve faltar exatamente 2 células: {missing:?}"
        );
    }
}

/// Boolean × Int com Ident: True x, False _ — Ident/Wildcard cobre __ANY__.
#[test]
fn exhaustiveness_boolean_int_with_ident() {
    let src = "\
f :: Boolean Int => Int\n\
lambda True x: x\n\
lambda False _: 0\n\
f True 42";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Boolean × Int sem Ident: True 0, False 1 — Int é __ANY__, Literal
/// não cobre. Deve dar NonExhaustiveMatch.
#[test]
fn exhaustiveness_boolean_int_without_ident() {
    let src = "\
f :: Boolean Int => Int\n\
lambda True 0: 0\n\
lambda False 1: 1\n\
f True 42";
    let err = infer_src_err(src);
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
}

/// 1 Boolean (degenera): True, False — idêntico ao comportamento atual.
#[test]
fn exhaustiveness_1_boolean_degenerates() {
    let src = "\
not :: Boolean => Boolean\n\
lambda True: False\n\
lambda False: True\n\
not True";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

/// 1 List (degenera): [], [h:t] — idêntico ao comportamento atual.
#[test]
fn exhaustiveness_1_list_degenerates() {
    let src = "\
len :: [Int] => Int\n\
lambda []: 0\n\
lambda [h : t]: + 1 (len t)\n\
len [1 2 3]";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// 3 Boolean exaustivo: 8 cláusulas cobrindo todas as combinações.
#[test]
fn exhaustiveness_3_boolean_exhaustive() {
    let src = "\
f3 :: Boolean Boolean Boolean => Boolean\n\
lambda True True True: True\n\
lambda True True False: False\n\
lambda True False True: False\n\
lambda True False False: False\n\
lambda False True True: False\n\
lambda False True False: False\n\
lambda False False True: False\n\
lambda False False False: False\n\
f3 True True True";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

/// 3 Boolean não-exaustivo: 4 cláusulas — faltam 4 células.
#[test]
fn exhaustiveness_3_boolean_non_exhaustive() {
    let src = "\
f3 :: Boolean Boolean Boolean => Boolean\n\
lambda True True True: True\n\
lambda True True False: False\n\
lambda True False True: False\n\
lambda True False False: False\n\
f3 True True True";
    let err = infer_src_err(src);
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
    if let kata_diagnostics::MiddleError::NonExhaustiveMatch { missing, .. } = err {
        assert_eq!(
            missing.len(),
            4,
            "deve faltar exatamente 4 células: {missing:?}"
        );
    }
}

/// Tuple como parâmetro: (a, b) com Ident — Tuple é átomo (__ANY__),
/// Ident cobre. Deve passar.
#[test]
fn exhaustiveness_tuple_as_atom() {
    let src = "\
fst :: (Int, Int) => Int\n\
lambda (a, b): a\n\
fst (1, 2)";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}
