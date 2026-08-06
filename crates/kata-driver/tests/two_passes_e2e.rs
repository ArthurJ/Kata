//! Testes E2E do ciclo de dois passes (Fase 4 — arity-uniformization).
//!
//! Valida que `kata run` e `kata eval` executam o ciclo de dois passes:
//! Pass 1 (parse_decls_only → resolve → extract_arities) → Pass 2
//! (parse_with_arity → resolve → infer → codegen).
//!
//! Testa via subprocesso `kata run` e `kata eval` — o caminho real do driver.

use std::fs;
use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e-arity");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

fn run_kata_file(file: &str) -> (String, String, i32) {
    let result = Command::new(kata_bin())
        .args(["run", file])
        .output()
        .expect("executar kata run");
    (
        String::from_utf8_lossy(&result.stdout).to_string(),
        String::from_utf8_lossy(&result.stderr).to_string(),
        result.status.code().unwrap_or(-1),
    )
}

fn eval_kata(expr: &str) -> (String, String, i32) {
    let result = Command::new(kata_bin())
        .args(["eval", expr])
        .output()
        .expect("executar kata eval");
    (
        String::from_utf8_lossy(&result.stdout).to_string(),
        String::from_utf8_lossy(&result.stderr).to_string(),
        result.status.code().unwrap_or(-1),
    )
}

// ── Sub-aplicação via kata run (DoD Fase 4) ─────────────────────

/// `+ 5 * 2 2` deve retornar 9 via `kata run` — o ciclo de dois passes
/// extrai aridades do prelude e do módulo, depois parseia arity-aware.
#[test]
fn kata_run_sub_aplicacao() {
    let path = write_temp_kata("sub_aplicacao", "+ 5 * 2 2");
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "9", "+ 5 * 2 2 deve imprimir 9");
}

/// `* 5 + 2 2` deve retornar 20 via `kata run`.
#[test]
fn kata_run_sub_aplicacao_aninhada() {
    let path = write_temp_kata("sub_aninhada", "* 5 + 2 2");
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "20", "* 5 + 2 2 deve imprimir 20");
}

/// `+ 1 2 3` com aridade 2 deve dar erro de parser via `kata run`.
#[test]
fn kata_run_excesso_posicional() {
    let path = write_temp_kata("excesso", "+ 1 2 3");
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "kata run deve falhar com erro de parser");
    assert!(
        stderr.contains("aridade padrão 2") || stderr.contains("excesso"),
        "erro deve mencionar aridade padrão 2 ou excesso, got: {stderr}"
    );
    let _ = stdout;
}

/// Função do usuário com aridade conhecida via `kata run`.
/// `soma :: Int Int => Int` + `soma 3 * 4 5` → 23
#[test]
fn kata_run_funcao_usuario_com_sub_aplicacao() {
    let src = "soma :: Int Int => Int\nlambda a b: + a b\nsoma 3 * 4 5";
    let path = write_temp_kata("fn_usuario", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "23", "soma 3 * 4 5 deve imprimir 23");
}

// ── Sub-aplicação via kata eval ──────────────────────────────────

/// `kata eval "+ 5 * 2 2"` deve retornar 9.
#[test]
fn kata_eval_sub_aplicacao() {
    let (stdout, stderr, code) = eval_kata("+ 5 * 2 2");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "9", "+ 5 * 2 2 deve imprimir 9");
}

/// `kata eval "+ 1 2 3"` deve dar erro de parser.
#[test]
fn kata_eval_excesso_posicional() {
    let (stdout, stderr, code) = eval_kata("+ 1 2 3");
    assert_ne!(code, 0, "kata eval deve falhar com erro de parser");
    assert!(
        stderr.contains("aridade padrão 2") || stderr.contains("excesso"),
        "erro deve mencionar aridade padrão 2 ou excesso, got: {stderr}"
    );
    let _ = stdout;
}

/// `kata eval "+ 1 2"` deve retornar 3 (aridade simples).
#[test]
fn kata_eval_aridade_simples() {
    let (stdout, stderr, code) = eval_kata("+ 1 2");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "3", "+ 1 2 deve imprimir 3");
}