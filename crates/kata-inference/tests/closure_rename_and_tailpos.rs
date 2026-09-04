//! Testes de renomeação Apply → Closure e propagação tail_pos.

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

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── Renomeação Apply → Closure ────────────────────────────────────

#[test]
fn closure_rename_int_add() {
    let tmod = infer_src("+ 1 2");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_add"));
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}

// ── tail_pos propagação ───────────────────────────────────────────

#[test]
fn match_tail_pos_propagated() {
    // match em tail position — body de cada arm é tail_pos = true.
    let tmod = infer_src("match True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert!(entry.tail_pos, "entry point é tail_pos");
    match &entry.kind {
        TypedExprKind::Match { arms, .. } => {
            for arm in arms {
                assert!(
                    arm.body.node.tail_pos,
                    "body de match arm em tail_pos deve ser true"
                );
            }
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

#[test]
fn let_value_not_tail_pos() {
    // constant value é tail_pos = false.
    // constant é ConstantBinding em tmod.constants.
    let tmod = infer_src("constant x := 42\n42");
    let binding = &tmod.constants[0];
    match &binding.node.kind {
        TypedExprKind::ConstantBinding { value, .. } => {
            assert!(
                !value.node.tail_pos,
                "constant value deve ser tail_pos = false"
            );
        }
        other => panic!("expected ConstantBinding, got {other:?}"),
    }
}

#[test]
fn apply_args_not_tail_pos() {
    // Argumentos de Apply são tail_pos = false.
    let tmod = infer_src("+ 1 2");
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::Closure { args, .. } => {
            for arg in args {
                assert!(
                    !arg.node.tail_pos,
                    "argumento de Closure deve ser tail_pos = false"
                );
            }
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}
