//! Testes E2E de proibição de recursão em Actions (DoD 31).
//!
//! Valida DoD 31: `RecursiveAction` error se Action chama a si mesma
//! (direta ou indiretamente). Testes usam `infer_module` diretamente
//! porque o erro é capturado no typeck, antes do codegen.

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

/// Roda o pipeline até infer_module e retorna o erro (se houver).
fn infer_err(src: &str) -> MiddleError {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("deve produzir erro de recursão")
}

/// Roda o pipeline até infer_module e verifica sucesso.
fn infer_ok(src: &str) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("deve passar sem erro de recursão");
}

// ── Teste 1: Recursão direta — A chama A ──────────────────────────────

/// `action a => Int\n    a!()` → RecursiveAction.
#[test]
fn recursao_direta() {
    let src = "action a => Int\n    a!()\na!()";
    let err = infer_err(src);
    assert!(
        matches!(err, MiddleError::RecursiveAction { .. }),
        "esperado RecursiveAction, got {err:?}"
    );
}

// ── Teste 2: Recursão indireta — A→B→A ───────────────────────────────

/// A chama B, B chama A → ciclo A → B → A.
#[test]
fn recursao_indireta_a_b_a() {
    let src = "action a => Int\n    b!()\naction b => Int\n    a!()\na!()";
    let err = infer_err(src);
    assert!(
        matches!(err, MiddleError::RecursiveAction { .. }),
        "esperado RecursiveAction, got {err:?}"
    );
    // Verifica que a cycle string contém os dois nomes.
    if let MiddleError::RecursiveAction { cycle, .. } = &err {
        assert!(
            cycle.contains("a") && cycle.contains("b"),
            "cycle deve conter 'a' e 'b': {cycle}"
        );
    }
}

// ── Teste 3: Sem recursão — A→B→C (sem ciclo) ────────────────────────

/// A chama B, B chama C, C retorna 7. Sem ciclo — deve passar.
#[test]
fn sem_recursao_cadeia_linear() {
    let src = "action c => Int\n    7\naction b => Int\n    c!()\naction a => Int\n    b!()\na!()";
    infer_ok(src);
}

// ── Teste 4: Action sem chamadas ──────────────────────────────────────

/// `action simples => Int\n    42` — sem chamadas, sem recursão.
#[test]
fn action_sem_chamadas() {
    let src = "action simples => Int\n    42\nsimples!()";
    infer_ok(src);
}

// ── Teste 5: Action chama função pura — não é recursão ───────────────

/// Action chama função pura (Closure, não ActionCall) — não conta.
#[test]
fn action_chama_funcao_pura() {
    // Usa função nomeada do prelude (+ _ 1) aplicada na Action.
    // `+` é uma função pura — ActionCall só acontece com `!`.
    let src = r#"+ 5 1
action a => Int
    + 5 1
a!()"#;
    infer_ok(src);
}

// ── Teste 6: FFI builtin (echo!) não conta como recursão ─────────────

/// Action chama echo! (builtin FFI) — não entra no call graph.
#[test]
fn ffi_builtin_nao_conta() {
    let src = r#"action a => Unit
    echo!("hello")
a!()"#;
    infer_ok(src);
}

// ── Teste 7: Recursão em Action com params ───────────────────────────

/// `action fat (n::Int) => Int\n    fat!(n)` → RecursiveAction.
#[test]
fn recursao_com_params() {
    let src = "action fat (n::Int) => Int\n    fat!(0)\nfat!(5)";
    let err = infer_err(src);
    assert!(
        matches!(err, MiddleError::RecursiveAction { .. }),
        "esperado RecursiveAction, got {err:?}"
    );
}

// ── Teste 8: Ciclo de 3 — A→B→C→A ────────────────────────────────────

/// A chama B, B chama C, C chama A → ciclo A → B → C → A.
#[test]
fn ciclo_de_tres() {
    let src =
        "action a => Int\n    b!()\naction b => Int\n    c!()\naction c => Int\n    a!()\na!()";
    let err = infer_err(src);
    assert!(
        matches!(err, MiddleError::RecursiveAction { .. }),
        "esperado RecursiveAction, got {err:?}"
    );
    if let MiddleError::RecursiveAction { cycle, .. } = &err {
        assert!(
            cycle.contains("a") && cycle.contains("b") && cycle.contains("c"),
            "cycle deve conter 'a', 'b' e 'c': {cycle}"
        );
    }
}
