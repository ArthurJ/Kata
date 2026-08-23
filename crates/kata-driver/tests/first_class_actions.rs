//! Testes E2E — First-class Actions: recursão indireta via parâmetro.
//!
//! PRD §4.3: interprocedural def-use for indirect invocation. When
//! `dispatcher!(worker_a, 42)` passes `worker_a` as param `job :: Action(..)`
//! and `dispatcher` calls `job!(payload)`, the call graph needs edge
//! `dispatcher → worker_a`. If worker_a calls dispatcher, there's a cycle.

use kata_diagnostics::MiddleError;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{DirectiveRegistry, ResolvedModule, load_stdlib_for_tests, resolve};

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
        directive_registry: DirectiveRegistry::new(),
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

/// Roda o pipeline até infer_module e verifica sucesso.
fn infer_ok(src: &str) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let _ = infer_module(&module, &resolved).expect("deve passar sem erro");
}

// ── Test 1: Recursão indireta via param — a → b → a ──────────────────

/// `a(f :: Action(Int) -> Unit)` calls `f!(1)`, and `b(n :: Int)` calls
/// `a!(b)`. This creates an indirect cycle: a → b → a (via param f).
/// Should fail with RecursiveAction.
#[test]
fn indirect_recursion_detected() {
    let src = "
action a (f :: Action(Int) -> Unit) => Unit
    f!(1)

action b (n :: Int) => Unit
    a!(b)

b!(0)
";
    let err = infer_err(src);
    assert!(
        matches!(err, MiddleError::RecursiveAction { .. }),
        "esperado RecursiveAction, got {err:?}"
    );
}

// ── Test 2: Dispatch/strategy pattern — sem falsa recursão ───────────

/// `dispatcher(job :: Action(Int) -> Unit, payload :: Int)` calls `job!(payload)`.
/// `worker_a` and `worker_b` are passed as first-class Action refs.
/// No cycle: dispatcher → worker_a, dispatcher → worker_b, but workers
/// don't call back dispatcher. Should compile fine.
#[test]
fn dispatch_no_false_recursion() {
    let src = "
action dispatcher (job :: Action(Int) -> Unit, payload :: Int) => Unit
    job!(payload)

action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    dispatcher!(worker_a, 42)
    dispatcher!(worker_b, 42)

main!()
";
    infer_ok(src);
}

// ── E2E helpers: compile + run via `kata run` ───────────────────────

use std::fs;
use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Cria um arquivo `.kata` temporário e retorna o path.
fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-first-class-actions");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata run <path>` e retorna (stdout, stderr, exit_code).
fn run_kata(path: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .args(["run", path])
        .output()
        .expect("executar kata run");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ── Test 3: E2E dispatch/strategy — compila, roda, imprime 43 e 44 ──

/// PRD §13.1: dispatch/strategy pattern completo. Compila, executa via
/// `kata run`, e verifica stdout contém "43" e "44".
#[test]
fn e2e_dispatch_strategy_runs() {
    let src = r#"action dispatcher (job :: Action(Int) -> Unit, payload :: Int) => Unit
    job!(payload)

action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    dispatcher!(worker_a, 42)
    dispatcher!(worker_b, 42)

main!()"#;
    let path = write_temp_kata("e2e_dispatch_strategy", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("43"),
        "deve imprimir 43 — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("44"),
        "deve imprimir 44 — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 4: E2E match selection — compila, roda, imprime 43 ─────────

/// PRD §13.3: match seleciona Action em runtime. `cond = True` seleciona
/// `worker_a`, `f!(42)` invoca indiretamente. Deve imprimir 43.
#[test]
fn e2e_match_selection_runs() {
    let src = r#"action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    let cond := True
    let f := match cond
        True: worker_a
        False: worker_b
    f!(42)

main!()"#;
    let path = write_temp_kata("e2e_match_selection", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("43"),
        "deve imprimir 43 — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 5: DoD 6 — fork!(f, (42,)) where f := worker ──────────────

/// PRD DoD 6: `fork!(f, (42,))` where `f := worker` works (fn_ptr from
/// variável, não de Ident direto). Compila e roda sem erro.
#[test]
fn e2e_fork_with_action_var() {
    let src = r#"action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    let f := worker
    fork!(f, (42,))

main!()"#;
    let path = write_temp_kata("e2e_fork_action_var", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir 42 — stdout: {stdout} | stderr: {stderr}"
    );
}
