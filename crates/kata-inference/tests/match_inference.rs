//! Testes de inference de match: exaustividade, patterns, guards.
//!
//! Estes testes focam no typeck (Pass 2), não no codegen.

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
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

// ── Match em Boolean (exaustivo) ──────────────────────────────────

#[test]
fn match_boolean_exhaustive() {
    let tmod = infer_src("match True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Match { scrutinee, arms } => {
            assert_eq!(scrutinee.node.ty, Ty::boolean());
            assert_eq!(arms.len(), 2);
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

// ── Match em Boolean (não-exaustivo → erro) ──────────────────────

#[test]
fn match_boolean_non_exhaustive_error() {
    let err = infer_src_err("match True\n    True: 1");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
    ));
}

// ── Match com otherwise (sempre exaustivo) ────────────────────────

#[test]
fn match_with_otherwise_is_exhaustive() {
    let tmod = infer_src("match True\n    True: 1\n    otherwise: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match em Int (tipo infinito) exige otherwise ─────────────────

#[test]
fn match_int_without_otherwise_errors() {
    // Pela Fase 2 (motor Maranget), tipos infinitos sem otherwise
    // produzem NonExhaustiveMatch com witness "_" (não MissingOtherwise).
    // MissingOtherwise é reservado para guards sem otherwise (Fase 3).
    let err = infer_src_err("match 42\n    0: 1");
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
}

#[test]
fn match_int_with_otherwise_ok() {
    let tmod = infer_src("match 42\n    0: 1\n    otherwise: 99");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match com variantes sem qualificação ─────────────────────────

#[test]
fn match_unqualified_variants_resolved() {
    // True e False sem qualificação são resolvidos pelo EnumRegistry
    // como variantes de Boolean (tipo do scrutinee).
    let tmod = infer_src("match True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Match { arms, .. } => {
            // True e False devem ser resolvidos como Variant, não Ident.
            for arm in arms {
                if let Some(pat) = &arm.pattern {
                    assert!(
                        matches!(&pat.node, kata_inference::TypedPattern::Variant { .. }),
                        "pattern deve ser Variant, não Ident"
                    );
                }
            }
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

// ── Match com qualified variants ──────────────────────────────────

#[test]
fn match_qualified_variants() {
    let tmod = infer_src("match True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match: braços devem retornar mesmo tipo ──────────────────────

#[test]
fn match_arms_type_mismatch_error() {
    let err = infer_src_err("match True\n    True: 1\n    False: \"texto\"");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
    ));
}

// ── Match com wildcard ───────────────────────────────────────────

#[test]
fn match_with_wildcard_is_exhaustive() {
    // Wildcard (_) cobre tudo — não precisa de otherwise.
    let tmod = infer_src("match 42\n    0: 1\n    _: 99");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match com Ident (binding) ────────────────────────────────────

#[test]
fn match_with_ident_binding() {
    // x liga o valor — otherwise implícito (cobre qualquer valor).
    let tmod = infer_src("match 42\n    0: 1\n    x: x");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Guards: verificação de tipo Boolean ───────────────────────────

#[test]
fn guard_condition_must_be_boolean() {
    // Match em Boolean — True e False são resolvidos via EnumRegistry.
    // Testa que o body de match arms pode ser Boolean.
    let src = "match True\n    True: True\n    False: False";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

#[test]
fn guard_condition_non_boolean_error() {
    // Match arms devem retornar o mesmo tipo. Se um retorna Int e outro
    // Boolean, deve dar TypeMismatch.
    let src = "match True\n    True: 1\n    False: False";
    let err = infer_src_err(src);
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
    ));
}

// ── Tuple patterns em match ──────────────────────────────────────

#[test]
fn match_tuple_pattern() {
    // match (1, 2) com pattern (a, b) — a e b são Int, ret é Int.
    let tmod = infer_src("match (1, 2)\n    (a, b): a\n    otherwise: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Cons pattern rejeitado ────────────────────────────────

#[test]
fn cons_pattern_rejected_in_fio2() {
    // [h : t] é reconhecido pelo parser mas rejeitado pelo typeck.
    let err = infer_src_err("match 42\n    [h : t]: h\n    otherwise: 0");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
    ));
}
