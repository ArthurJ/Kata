//! Testes E2E de validação de `refines`: a interface delegada deve
//! existir no InterfaceRegistry (não pode ser família ou tipo concreto).
//!
//! Estes testes rodam o pipeline completo (lex → parse → resolve → merge
//! prelude → infer) para validar que `refines` com uma não-interface
//! produz erro claro em infer_module.

use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{load_stdlib_for_tests, resolve};

fn merge_resolved(
    prelude: kata_resolution::ResolvedModule,
    user: kata_resolution::ResolvedModule,
) -> kata_resolution::ResolvedModule {
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
    kata_resolution::ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
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

/// Helper: roda o pipeline completo e retorna o erro de infer (se houver).
fn infer_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).unwrap_err()
}

/// Helper: roda o pipeline completo e retorna Ok (se sucesso).
fn infer_ok(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("infer deve succeed")
}

/// `refines` com nome AllCaps que não é interface registrada — deve falhar
/// com mensagem indicando que não é uma interface.
#[test]
fn refines_unknown_allcaps_name_fails() {
    let src = r#"
data (Int, > _ 0) as PositiveInt
PositiveInt refines BOGUS
1
"#;
    let err = infer_err(src);
    let msg = err.to_string();
    assert!(
        msg.contains("não é uma interface"),
        "esperava mensagem sobre não-interface, got: {msg}"
    );
    assert!(
        msg.contains("BOGUS"),
        "esperava nome BOGUS na mensagem: {msg}"
    );
}

/// `refines` com interface válida (NUM) — deve passar sem erro.
#[test]
fn refines_valid_interface_ok() {
    let src = r#"
data (Int, > _ 0) as PositiveInt
PositiveInt refines NUM
1
"#;
    infer_ok(src);
}
