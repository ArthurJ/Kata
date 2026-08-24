//! Testes E2E — First-class Actions: proibição de Ty::Action em data e canal.
//!
//! PRD §3.7: o typeck rejeita `Ty::Action` em posições de `data` e canal.
//! Actions são comportamento, não informação — não podem viver em structs
//! nem viajar por canais.

use kata_diagnostics::MiddleError;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

/// Combina prelude + módulo do usuário (replica do driver).
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
        type_graph: prelude.type_graph.clone(),
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

/// Roda o pipeline até infer_module e retorna o erro.
fn infer_err(src: &str) -> MiddleError {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("deve produzir erro")
}

// ── Test 1: Action em campo de data ──────────────────────────────────

#[test]
fn action_em_campo_de_data_erro() {
    let src = r#"action worker (n :: Int) => Unit
    echo!(n)
data Wrapper (job :: Action(Int) -> Unit)
"#;
    let err = infer_err(src);
    assert!(
        matches!(err, MiddleError::TypeMismatch { ref found, .. } if found.contains("Action não é permitida em data")),
        "deve rejeitar Action em data, foi: {err:?}"
    );
}

// ── Test 2: Action em channel send ───────────────────────────────────

#[test]
fn action_em_channel_send_erro() {
    let src = r#"action worker (n :: Int) => Unit
    echo!(n)
action main => Unit
    let (tx, rx) := channel!()
    tx <! worker
"#;
    let err = infer_err(src);
    assert!(
        matches!(err, MiddleError::TypeMismatch { ref found, .. } if found.contains("Action não é permitida em canal")),
        "deve rejeitar Action em canal, foi: {err:?}"
    );
}

// ── Test 3: Action como argumento de função pura ────────────────────
//
// Nota: Este teste é difícil de disparar diretamente na linguagem atual
// porque Actions são referenciadas por nome e o typeck ainda não expõe
// um caminho natural onde uma função pura receba uma Action como argumento
// sem antes falhar em outro check. Deixamos o teste comentado como
// placeholder — a verificação em apply.rs (reject_action_arg_for_pure_fn)
// protege o caso em que uma função pura (is_action: false) receba
// um argumento tipado como Ty::Action(..).
