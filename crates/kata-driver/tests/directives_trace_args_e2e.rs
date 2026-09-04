//! Testes E2E — Diretivas customizadas: args no site de aplicação e @log do stdlib.
//!
//! Valida que diretivas com parâmetros (msg, when, topic, policy) despacham
//! corretamente por arg_keys, e que @log do stdlib funciona sem declaration local.

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
    let dir = std::env::temp_dir().join("kata-driver-directives-e2e");
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

// ── Fase 1: Args no site de aplicação + _args em funções ────────────

// ── Test 21: @log{msg: Text, when: "enter"} em função pura com _args ─

#[test]
fn e2e_trace_args_function_pure() {
    // Função pura com diretiva Enter: _log_publish! é FFI direta (não bloqueia).
    // Não podemos verificar o log (precisa de consumidor log_recv!()),
    // mas verificamos que a função compila e executa corretamente.
    let src = r#"directive trace_test{when: Hook::Enter, on: Target::Function, msg: Text}
    _log_publish!(_log_tag(LogLevel::Info), format!(_msg, (_name,)))

@trace_test{msg: "entering {}", when: "enter"}
dobro :: Int => Int
lambda n: * n 2

echo!(dobro 21)"#;
    let path = write_temp_kata("e2e_trace_args_fn", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir resultado 42 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 22: @log{msg: Text, when: "enter"} em action com _args ────

#[test]
fn e2e_trace_args_action() {
    let src = r#"directive trace_test{when: Hook::Enter, on: Target::Action, msg: Text}
    echo!(format!(_msg, (_name,)))

@trace_test{msg: "action {}", when: "enter"}
action processar(x :: Int) => Int
    + x 1

echo!(processar!(5))"#;
    let path = write_temp_kata("e2e_trace_args_act", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("action processar"),
        "deve imprimir msg formatada com _name - stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("6"),
        "deve imprimir resultado 6 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 23: Despacho por arg_keys — msg vs msg+topic ───────────────

#[test]
fn e2e_trace_dispatch_by_arg_keys() {
    let src = r#"directive trace_test{when: Hook::Enter, on: Target::Action, msg: Text}
    echo!(format!(_msg, (_name,)))

directive trace_test{when: Hook::Enter, on: Target::Action, msg: Text, topic: Text}
    echo!(format!(_msg, (_name,)))

@trace_test{msg: "simple {}", when: "enter"}
action sem_topic(x :: Int) => Int
    + x 1

@trace_test{msg: "with topic {}", when: "enter", topic: "audit"}
action com_topic(x :: Int) => Int
    + x 2

echo!(sem_topic!(10))
echo!(com_topic!(20))"#;
    let path = write_temp_kata("e2e_trace_dispatch", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("simple sem_topic"),
        "deve despachar para overload sem topic - stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("with topic com_topic"),
        "deve despachar para overload com topic - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 24: Exit hook com args do site ─────────────────────────────

#[test]
fn e2e_trace_exit_args_function() {
    // Exit em função pura: _log_publish! com _return. Verifica que compila e executa.
    let src = r#"directive trace_test{when: Hook::Exit, on: Target::Function, msg: Text}
    _log_publish!(_log_tag(LogLevel::Info), format!(_msg, (_name, _return)))

@trace_test{msg: "exit {} -> {}", when: "exit"}
inc :: Int => Int
lambda n: + n 1

echo!(inc 41)"#;
    let path = write_temp_kata("e2e_trace_exit_fn", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir resultado 42 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 25: @log do stdlib sem declaration local (Fase 2 DoD) ────

#[test]
fn e2e_log_stdlib_function() {
    // @log do stdlib (core.kata) sem declaration local.
    // log!() vai para CSP (não stdout) — verificamos que compila e executa.
    let src = r#"@log{msg: "entering {_args}", when: "enter"}
dobra :: Int => Int
lambda n: * n 2

echo!(dobra 21)"#;
    let path = write_temp_kata("e2e_trace_stdlib_fn", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir resultado 42 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 26: @log do stdlib com exit hook (Fase 2 DoD) ────────────

#[test]
fn e2e_log_stdlib_exit() {
    let src = r#"@log{msg: "exit {_return}", when: "exit"}
inc :: Int => Int
lambda n: + n 1

echo!(inc 41)"#;
    let path = write_temp_kata("e2e_trace_stdlib_exit", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir resultado 42 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 27: @log do stdlib com topic+policy em action (Fase 2 DoD) ─

#[test]
fn e2e_log_stdlib_action_topic_policy() {
    // Action com topic+policy: log!() publica em CSP com policy "drop".
    let src = r#"@log{msg: "action {_args}", when: "enter", topic: "audit", policy: "drop"}
action processar(x :: Int) => Int
    + x 1

echo!(processar!(5))"#;
    let path = write_temp_kata("e2e_trace_stdlib_act", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("6"),
        "deve imprimir resultado 6 - stdout: {stdout} | stderr: {stderr}"
    );
}
