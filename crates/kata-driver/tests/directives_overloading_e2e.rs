//! Testes E2E — Diretivas customizadas: overloading (mesmo nome, hooks/targets diferentes).
//!
//! Valida que diretivas com o mesmo nome mas (Hook, Target) diferentes coexistem,
//! e que duplicatas exatas (mesmo nome + Hook + Target) produzem erro.

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

// ── Test 16: Overloading — mesmo nome, (Enter, Action) e (Enter, Function) ─

#[test]
fn e2e_overloading_enter_action_and_function() {
    let src = r#"directive trace{when: Hook::Enter, on: Target::Action}
    echo!("action-enter")

directive trace{when: Hook::Enter, on: Target::Function}
    let _ := _name

@trace
action act(x :: Int) => Int
    + x 1

@trace
double :: Int => Int
lambda x: * x 2

echo!(act!(5))
echo!(double 10)"#;
    let path = write_temp_kata("e2e_overloading_enter", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Action dispara Enter::Action (imprime "action-enter"), Function dispara Enter::Function (let _ := _name, puro)
    assert!(
        stdout.contains("action-enter"),
        "deve imprimir action-enter — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("6"),
        "deve imprimir 6 (act 5+1) — stdout: {stdout}"
    );
    assert!(
        stdout.contains("20"),
        "deve imprimir 20 (double 10*2) — stdout: {stdout}"
    );
}

// ── Test 17: Overloading — Target::Any casa com action e função ──────

#[test]
fn e2e_overloading_any_matches_both() {
    let src = r#"directive trace{when: Hook::Exit, on: Target::Any}
    let _ := _return

@trace
action act(x :: Int) => Int
    + x 1

@trace
double :: Int => Int
lambda x: * x 2

echo!(act!(5))
echo!(double 10)"#;
    let path = write_temp_kata("e2e_overloading_any", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Exit::Any dispara em ambos. O body da diretiva é `let _ := _return` (puro).
    // Resultados: act(5) = 6, double(10) = 20
    assert!(
        stdout.contains("6"),
        "deve imprimir 6 — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("20"),
        "deve imprimir 20 — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 18: Overloading — (nome, when, on) duplicado → erro ──────────

#[test]
fn e2e_overloading_duplicate_error() {
    let src = r#"directive trace{when: Hook::Enter, on: Target::Action}
    echo!("first")

directive trace{when: Hook::Enter, on: Target::Action}
    echo!("second")

@trace
action act(x :: Int) => Int
    + x 1

echo!(act!(5))"#;
    let path = write_temp_kata("e2e_overloading_dup_error", src);
    let (_stdout, stderr, code) = run_kata(&path);
    assert_ne!(
        code, 0,
        "deve falhar (diretiva duplicada) - stderr: {stderr}"
    );
    assert!(
        stderr.contains("duplicada") || stderr.contains("DuplicateDirective"),
        "deve reportar erro de diretiva duplicada - stderr: {stderr}"
    );
}

// ── Test 19: Overloading — mesmo nome com hooks diferentes coexiste ─

#[test]
fn e2e_overloading_different_hooks_coexist() {
    let src = r#"directive trace{when: Hook::Enter, on: Target::Any}
    echo!("ENTER")

directive trace{when: Hook::Exit, on: Target::Any}
    echo!("EXIT")

@trace
action act(x :: Int) => Int
    + x 1

echo!(act!(5))"#;
    let path = write_temp_kata("e2e_overloading_diff_hooks", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("ENTER"),
        "deve imprimir ENTER - stdout: {stdout}"
    );
    assert!(
        stdout.contains("EXIT"),
        "deve imprimir EXIT - stdout: {stdout}"
    );
    let enter_pos = stdout.find("ENTER");
    let exit_pos = stdout.find("EXIT");
    assert!(
        enter_pos < exit_pos,
        "ENTER deve vir antes de EXIT - stdout: {stdout}"
    );
}
