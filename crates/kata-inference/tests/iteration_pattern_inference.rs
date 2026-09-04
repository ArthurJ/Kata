//! Testes de iteração for-in e pattern matching em listas (DoDs 28-29).

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, TypedPattern, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

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
    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);

    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
        refines_registry,
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

fn entry(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── DoD 28: `for x in [1 2 3]` define `x: Int` no escopo do body ──────
//
// `for` só existe em Action body. Criamos uma action com `for x in [1 2 3]`
// e verificamos que var_ty = Int no ForIn.

#[test]
fn dod28_for_in_defines_x_int() {
    let src = "action iterar => Int\n    var total := 0\n    for x in [1 2 3]\n        total := x\n    return total\n0";
    let typed = infer_src(src);
    // O for deve estar no body da action `iterar` (não actions[0],
    // que pode ser uma action do prelude como echo/_print).
    let action = typed
        .actions
        .iter()
        .find(|a| a.name == "iterar")
        .expect("action `iterar` deve existir");
    // Procura o ForIn no body da action.
    let for_in = action
        .body
        .iter()
        .find(|s| matches!(&s.node.kind, TypedExprKind::ForIn { .. }))
        .expect("action body deve conter ForIn");
    match &for_in.node.kind {
        TypedExprKind::ForIn {
            var_name, var_ty, ..
        } => {
            assert_eq!(var_name, "x", "var_name deve ser 'x'");
            assert_eq!(*var_ty, Ty::int(), "var_ty deve ser Int");
        }
        other => panic!("esperado ForIn, encontrado {other:?}"),
    }
}

// ── DoD 29: `[h : t]` pattern match em List: `h: Int, t: List(Int)` ──

#[test]
fn dod29_pattern_cons_match() {
    let src = "constant lst := [1 2 3]\nmatch lst\n    [h : t]: h\n    otherwise: 0";
    let typed = infer_src(src);
    let e = entry(&typed);
    // O entry deve ser um Match com um arm Cons.
    match &e.kind {
        TypedExprKind::Match { arms, .. } => {
            assert!(!arms.is_empty(), "deve ter ao menos 1 arm");
            // O primeiro arm deve ter pattern Cons com head: Int, tail: List(Int).
            let arm = &arms[0];
            let pattern = arm
                .pattern
                .as_ref()
                .expect("arm deve ter pattern (não otherwise)");
            match &pattern.node {
                TypedPattern::Cons { head, tail } => {
                    match &head.node {
                        TypedPattern::Ident { name, ty } => {
                            assert_eq!(name, "h");
                            assert_eq!(*ty, Ty::int(), "head deve ser Int");
                        }
                        other => panic!("head do Cons deve ser Ident, encontrado {other:?}"),
                    }
                    match &tail.node {
                        TypedPattern::Ident { name, ty } => {
                            assert_eq!(name, "t");
                            assert_eq!(
                                *ty,
                                Ty::List(Box::new(Ty::int())),
                                "tail deve ser List(Int)"
                            );
                        }
                        other => panic!("tail do Cons deve ser Ident, encontrado {other:?}"),
                    }
                }
                other => panic!("pattern deve ser Cons, encontrado {other:?}"),
            }
        }
        other => panic!("entry deve ser Match, encontrado {other:?}"),
    }
}
