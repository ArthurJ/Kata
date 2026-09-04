//! Testes E2E — Diretivas customizadas: variáveis de reflexão e target mismatch.
//!
//! Valida variáveis de reflexão (_name, _arity, _is_action) e erros de
//! target mismatch (Target::Action em função e vice-versa).

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

// ── Test 8: Reflection vars — _name, _arity, _is_action ─────────────

#[test]
fn e2e_reflection_vars() {
    let src = r#"directive trace_meta{when: Hook::Enter, on: Target::Action}
    echo!(_name)
    echo!(_arity)
    echo!(_is_action)

@trace_meta
action greet(name :: Text) => Unit
    echo!("hello")

greet!("world")"#;
    let path = write_temp_kata("e2e_reflection_vars", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("greet"),
        "deve imprimir 'greet' (_name) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("1"),
        "deve imprimir 1 (_arity) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("True"),
        "deve imprimir True (_is_action) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 9: Enter em função pura ─────────────────────────────────────

#[test]
fn e2e_enter_function() {
    let src = r#"directive trace_fn{when: Hook::Enter, on: Target::Function}
    let _ := _name

@trace_fn
double :: Int => Int
lambda x: * x 2

echo!(double 10)"#;
    let path = write_temp_kata("e2e_enter_fn", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // A diretiva inlina `let _ := _name` (binding de reflexão) antes do body.
    // A função executa corretamente: double(10) = 20.
    assert!(
        stdout.contains("20"),
        "deve imprimir 20 (resultado) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 10: Target mismatch — on: Action aplicada em função → erro ─

#[test]
fn e2e_target_mismatch_action_on_function() {
    let src = r#"directive trace_act{when: Hook::Enter, on: Target::Action}
    echo!(_name)

@trace_act
double :: Int => Int
lambda x: * x 2

echo!(double 10)"#;
    let path = write_temp_kata("e2e_target_mismatch_act_on_fn", src);
    let (_stdout, stderr, code) = run_kata(&path);
    assert_ne!(
        code, 0,
        "deve falhar (Target::Action em função) — stderr: {stderr}"
    );
    assert!(
        stderr.contains("não pode decorar")
            || stderr.contains("DirectiveTargetMismatch")
            || stderr.contains("target"),
        "deve reportar erro de target mismatch — stderr: {stderr}"
    );
}

#[test]
fn e2e_target_mismatch_function_on_action() {
    let src = r#"directive trace_fn{when: Hook::Enter, on: Target::Function}
    let _ := _name

@trace_fn
action greet(name :: Text) => Unit
    echo!("hello")

greet!("world")"#;
    let path = write_temp_kata("e2e_target_mismatch_fn_on_act", src);
    let (_stdout, stderr, code) = run_kata(&path);
    assert_ne!(
        code, 0,
        "deve falhar (Target::Function em action) — stderr: {stderr}"
    );
    assert!(
        stderr.contains("não pode decorar")
            || stderr.contains("DirectiveTargetMismatch")
            || stderr.contains("target"),
        "deve reportar erro de target mismatch — stderr: {stderr}"
    );
}
