//! Testes de inference de lambdas anônimos.
//!
//! Após DoD 30, lambdas sem contexto de tipo produzem LambdaInferenceFail.
//! Os testes usam ascription `(lambda ...)::(Int -> Int)` para fornecer o tipo.

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

// ── Lambda anônimo: tipo Function ────────────────────────────────

#[test]
fn lambda_anon_has_function_type() {
    // (lambda x: x)::(Int -> Int) — identidade com hint top-down.
    let tmod = infer_src("(lambda x: x)::(Int -> Int)");
    let entry = entry_typed(&tmod);
    match &entry.ty {
        Ty::Function(params, ret) => {
            assert_eq!(params.len(), 1);
            assert_eq!(**ret, Ty::int());
        }
        other => panic!("expected Function type, got {other:?}"),
    }
    match &entry.kind {
        TypedExprKind::TypeAscription { expr, .. } => match &expr.node.kind {
            TypedExprKind::Grouping { inner } => match &inner.node.kind {
                TypedExprKind::Lambda {
                    param_types,
                    clauses,
                    ..
                } => {
                    assert_eq!(param_types.len(), 1);
                    assert_eq!(clauses.len(), 1, "lambda anônimo tem 1 cláusula");
                }
                other => panic!("expected Lambda inside grouping, got {other:?}"),
            },
            other => panic!("expected Grouping inside ascription, got {other:?}"),
        },
        other => panic!("expected TypeAscription, got {other:?}"),
    }
}

// ── Lambda anônimo com 2 parâmetros ──────────────────────────────

#[test]
fn lambda_anon_two_params() {
    let tmod = infer_src("(lambda a b: a)::(Int Int -> Int)");
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::TypeAscription { expr, .. } => match &expr.node.kind {
            TypedExprKind::Grouping { inner } => match &inner.node.kind {
                TypedExprKind::Lambda { param_types, .. } => {
                    assert_eq!(param_types.len(), 2);
                }
                other => panic!("expected Lambda inside grouping, got {other:?}"),
            },
            other => panic!("expected Grouping inside ascription, got {other:?}"),
        },
        other => panic!("expected TypeAscription, got {other:?}"),
    }
}

// ── Lambda como valor (call_indirect) ─────────────────────────────

#[test]
fn lambda_assigned_to_var_has_function_type() {
    // let f := (lambda x: x)::(Int -> Int)
    // f é Ty::Function no TypeEnv. O typeck aceita.
    let tmod = infer_src("f :: Int => Int\nlambda x: x\n42");
    let entry = entry_typed(&tmod);
    // Entry é 42 (Int) — o let define f mas entry é a última expr.
    assert_eq!(entry.ty, Ty::int());
    // f está no TypeEnv como Ty::Function.
    let f_ty = tmod.type_env.lookup("f");
    assert!(matches!(f_ty, Some(Ty::Function(_, _))));
}
