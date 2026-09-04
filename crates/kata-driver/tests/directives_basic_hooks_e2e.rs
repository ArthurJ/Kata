//! Testes E2E — Diretivas customizadas: hooks básicos (Enter, Exit, ShortCircuit, Transform).
//!
//! Valida que o desugaring de cada hook individual funciona end-to-end.

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

// ── Test 1: Enter em action — imprime nome ao entrar ────────────────

#[test]
fn e2e_enter_action_prints_name() {
    let src = r#"directive trace_enter{when: Hook::Enter, on: Target::Action}
    echo!(_name)

@trace_enter
action greet(name :: Text) => Unit
    echo!("hello")

greet!("world")"#;
    let path = write_temp_kata("e2e_enter_action", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("greet"),
        "deve imprimir 'greet' (nome da action) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("hello"),
        "deve imprimir 'hello' (body original) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 2: Enter com _args — acessa argumentos via tupla ────────────

#[test]
fn e2e_enter_action_args() {
    let src = r#"directive trace_args{when: Hook::Enter, on: Target::Action}
    echo!(_args.0)

@trace_args
action add(a :: Int, b :: Int) => Int
    + a b

echo!(add!(3, 4))"#;
    let path = write_temp_kata("e2e_enter_args", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // _args.0 = 3 (primeiro arg), depois o resultado 7
    assert!(
        stdout.contains("3"),
        "deve imprimir 3 (primeiro arg via _args.0) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("7"),
        "deve imprimir 7 (resultado de add) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 3: Exit em action — observa resultado ───────────────────────

#[test]
fn e2e_exit_action_observes_result() {
    let src = r#"directive trace_exit{when: Hook::Exit, on: Target::Any}
    echo!(_return)

@trace_exit
action double(x :: Int) => Int
    * x 2

echo!(double!(21))"#;
    let path = write_temp_kata("e2e_exit_action", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Exit observa _return = 42, depois o caller imprime 42
    // stdout deve conter 42 duas vezes (uma do echo! da diretiva, uma do echo! do caller)
    assert!(
        stdout.contains("42"),
        "deve imprimir 42 — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 4: ShortCircuit — prossegue quando None ──────────

#[test]
fn e2e_shortcircuit_proceeds() {
    let src = r#"directive gate{when: Hook::ShortCircuit, on: Target::Action}
    None

@gate
action process(x :: Int) => Int
    + x 1

echo!(process!(10))"#;
    let path = write_temp_kata("e2e_shortcircuit_proceeds", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // ShortCircuit retorna None → body executa → 10 + 1 = 11
    assert!(
        stdout.contains("11"),
        "deve imprimir 11 (body executou) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 5: ShortCircuit — short-circuita com Some ────────

#[test]
fn e2e_shortcircuit_blocks() {
    let src = r#"directive gate{when: Hook::ShortCircuit, on: Target::Action}
    Some(999)

@gate
action process(x :: Int) => Int
    + x 1

echo!(process!(10))"#;
    let path = write_temp_kata("e2e_shortcircuit_blocks", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // ShortCircuit retorna Some(999) → body não executa → resultado = 999
    assert!(
        stdout.contains("999"),
        "deve imprimir 999 (short-circuit) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        !stdout.contains("11"),
        "não deve imprimir 11 (body não executou) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 6: Transform — modifica o resultado ────────────────────────

#[test]
fn e2e_transform_modifies_result() {
    let src = r#"directive redact{when: Hook::Transform, on: Target::Action}
    + _return 1

@redact
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_transform", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Transform: _return = 10 + 1 = 11, depois Transform soma 1 → 12
    assert!(
        stdout.contains("12"),
        "deve imprimir 12 (transform: 11 + 1) — stdout: {stdout} | stderr: {stderr}"
    );
}
