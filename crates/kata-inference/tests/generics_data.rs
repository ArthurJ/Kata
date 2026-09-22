//! Testes E2E para generics paramétricos em `data`.
//!
//! Fase 3: smart constructor genérico, unify, bound check.

use kata_core::ty::TypeEnv;
use kata_inference::{TypedModule, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

// ── Helpers ───────────────────────────────────────────────────────

fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = TypeEnv::with_parent(prelude.type_env);
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
        embed_dependencies: Vec::new(),
    }
}

fn infer_src(src: &str) -> Result<TypedModule, kata_diagnostics::MiddleError> {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved)
}

// ── Testes ────────────────────────────────────────────────────────

/// `data Pair (first::T second::T) where T implements NUM`
/// Construtor aceita dois argumentos do mesmo tipo que implementa NUM.
/// `Pair 3 4` tipa como Pair::(Int, Int).
#[test]
fn generic_data_constructor_int() {
    let src = r#"
data Pair (first::T second::T) where T implements NUM
Pair 3 4
"#;
    let result = infer_src(src);
    assert!(
        result.is_ok(),
        "inferência deve succeed: {:?}",
        result.err()
    );
}

/// `Pair 3.0 4.0` tipa como Pair::(Float, Float).
#[test]
fn generic_data_constructor_float() {
    let src = r#"
data Pair (first::T second::T) where T implements NUM
Pair 3.0 4.0
"#;
    let result = infer_src(src);
    assert!(
        result.is_ok(),
        "inferência deve succeed: {:?}",
        result.err()
    );
}

/// `Pair "a" "b"` falha — Text não implementa NUM.
#[test]
fn generic_data_constructor_text_fails() {
    let src = r#"
data Pair (first::T second::T) where T implements NUM
Pair "a" "b"
"#;
    let result = infer_src(src);
    assert!(result.is_err(), "Text não implementa NUM — deve falhar");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("não implementa") || err.contains("NUM"),
        "erro deve mencionar NUM/não implementa, got: {err}"
    );
}

/// Type params livres (sem bound): `data Par (first::A second::B)`
/// aceita qualquer par de tipos.
#[test]
fn generic_data_free_params() {
    let src = r#"
data Par (first::A second::B)
Par 3 "hello"
"#;
    let result = infer_src(src);
    assert!(
        result.is_ok(),
        "inferência deve succeed: {:?}",
        result.err()
    );
}

/// Type params livres com tipos iguais: `Par 3 4` também funciona.
#[test]
fn generic_data_free_params_same_type() {
    let src = r#"
data Par (first::A second::B)
Par 3 4
"#;
    let result = infer_src(src);
    assert!(
        result.is_ok(),
        "inferência deve succeed: {:?}",
        result.err()
    );
}
