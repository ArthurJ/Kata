//! Testes de indexação e operador `in` (DoDs 30, 32, extra dot-n-on-range).

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
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

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn entry(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── DoD 30: `arr.0` em Array desugara para `at arr 0` → `Result::(Int, Err)`

#[test]
fn dod30_dot_n_array_desugars_to_at() {
    let src = "constant arr := {1 2 3}\narr.0";
    let typed = infer_src(src);
    let e = entry(&typed);
    // O entry deve ser um Closure (call de `at`) com ffi_symbol.
    match &e.kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            // callee deve ser Ident("at")
            match &callee.node.kind {
                TypedExprKind::Ident { name } => {
                    assert_eq!(name, "at", "callee deve ser 'at'");
                }
                other => panic!("callee deve ser Ident(\"at\"), encontrado {other:?}"),
            }
            // 2 args: receptor + índice
            assert_eq!(args.len(), 2, "deve ter 2 args (receptor + índice)");
            // O índice deve ser IntLit(0)
            match &args[1].node.kind {
                TypedExprKind::IntLit { text } => {
                    assert_eq!(text, "0", "índice deve ser 0");
                }
                other => panic!("segundo arg deve ser IntLit(0), encontrado {other:?}"),
            }
            // ffi_symbol deve ser Some (kata_rt_array_get_checked)
            assert!(
                ffi_symbol.is_some(),
                "ffi_symbol deve ser Some (dispatch INDEXABLE)"
            );
        }
        other => panic!("entry deve ser Closure (dispatch `at`), encontrado {other:?}"),
    }
    // O tipo deve ser Result::(Int, Text) — default do prelude preenche E|Text
    let expected = Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()]);
    assert_eq!(
        e.ty, expected,
        "tipo deve ser Result::(Int, Text), encontrado {:?}",
        e.ty
    );
}

// ── DoD 32: `3 in {1 2 3}` infere `Boolean` ───────────────────────────

#[test]
fn dod32_in_operator_infere_boolean() {
    let typed = infer_src("3 in {1 2 3}");
    let e = entry(&typed);
    assert!(
        matches!(&e.kind, TypedExprKind::In { .. }),
        "entry deve ser In, encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::Sum("Boolean".into()), "tipo deve ser Boolean");
}

// ── Extra: `.N` em Range é type error ─────────────────────────────────

#[test]
fn dot_n_on_range_is_type_error() {
    let err = infer_src_err("constant r := [0..1..10]\nr.0");
    // Deve falhar com NotIndexable ou TypeMismatch.
    let msg = format!("{err:?}");
    assert!(
        msg.contains("NotIndexable") || msg.contains("TypeMismatch"),
        "deve falhar com NotIndexable ou TypeMismatch, encontrado {msg}"
    );
}
