//! Testes E2E — Diretivas customizadas: stacking e ordenação de múltiplas diretivas.
//!
//! Valida que múltiplas diretivas aplicadas ao mesmo alvo executam na ordem correta
//! e que ShortCircuit interage corretamente com Exit.

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

// ── Test 7: Stacking — Enter + Exit na mesma action ─────────────────

#[test]
fn e2e_stacking_enter_exit() {
    let src = r#"directive trace_enter{when: Hook::Enter, on: Target::Any}
    echo!("ENTER")

directive trace_exit{when: Hook::Exit, on: Target::Any}
    echo!("EXIT")

@trace_enter
@trace_exit
action compute(x :: Int) => Int
    + x 2

echo!(compute!(5))"#;
    let path = write_temp_kata("e2e_stacking", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Enter imprime "ENTER", body executa (5+2=7), Exit imprime "EXIT", caller imprime 7
    assert!(
        stdout.contains("ENTER"),
        "deve imprimir ENTER — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("EXIT"),
        "deve imprimir EXIT — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("7"),
        "deve imprimir 7 (resultado) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 12: Stacking 3 diretivas — Enter + ShortCircuit + Exit ─────

#[test]
fn e2e_stacking_three_directives() {
    let src = r#"directive log_enter{when: Hook::Enter, on: Target::Any}
    echo!("ENTER")

directive log_exit{when: Hook::Exit, on: Target::Any}
    echo!("EXIT")

directive gate{when: Hook::ShortCircuit, on: Target::Action}
    None

@log_enter
@log_exit
@gate
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_stacking_three", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    let enter_pos = stdout.find("ENTER");
    let result_pos = stdout.find("11");
    let exit_pos = stdout.find("EXIT");
    assert!(
        enter_pos.is_some(),
        "deve imprimir ENTER — stdout: {stdout}"
    );
    assert!(exit_pos.is_some(), "deve imprimir EXIT — stdout: {stdout}");
    assert!(result_pos.is_some(), "deve imprimir 11 — stdout: {stdout}");
    assert!(
        enter_pos < exit_pos,
        "ENTER deve vir antes de EXIT — stdout: {stdout}"
    );
}

// ── Test 13: ShortCircuit interna + Exit externa — short-circuit propaga ─

#[test]
fn e2e_shortcircuit_inner_exit_outer() {
    let src = r#"directive log_exit{when: Hook::Exit, on: Target::Any}
    echo!(_return)

directive gate{when: Hook::ShortCircuit, on: Target::Action}
    Some(999)

@log_exit
@gate
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_sc_inner_exit_outer", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("999"),
        "deve imprimir 999 (short-circuit + exit observa) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        !stdout.contains("11"),
        "não deve imprimir 11 (body não executou) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 14: ShortCircuit interna + Exit externa — prossegue ─────────

#[test]
fn e2e_shortcircuit_inner_exit_outer_proceeds() {
    let src = r#"directive log_exit{when: Hook::Exit, on: Target::Any}
    echo!(_return)

directive gate{when: Hook::ShortCircuit, on: Target::Action}
    None

@log_exit
@gate
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_sc_inner_exit_outer_proceeds", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("11"),
        "deve imprimir 11 (body executou, exit observa) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 15: Stacking ordem Enter — primeira = mais externa ──────────

#[test]
fn e2e_stacking_enter_order() {
    let src = r#"directive outer{when: Hook::Enter, on: Target::Any}
    echo!("OUTER")

directive inner{when: Hook::Enter, on: Target::Any}
    echo!("INNER")

@outer
@inner
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_stacking_enter_order", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    let outer_pos = stdout.find("OUTER");
    let inner_pos = stdout.find("INNER");
    assert!(
        outer_pos.is_some(),
        "deve imprimir OUTER — stdout: {stdout}"
    );
    assert!(
        inner_pos.is_some(),
        "deve imprimir INNER — stdout: {stdout}"
    );
    assert!(
        outer_pos < inner_pos,
        "OUTER deve vir antes de INNER (primeira = mais externa) — stdout: {stdout}"
    );
}
