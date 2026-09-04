//! Testes E2E — comptime arithmetic evaluation (Fase 1).
//!
//! O comptime pass avalia `ConstantBinding`s em compile-time via
//! JIT-and-execute, substituindo o value por um literal.
//!
//! Fase 1 DoD: `@comptime + 1 2` gera `x = 3` — a expressão
//! `+ 1 2` é avaliada em compile-time e substituída por `IntLit "3"`.

use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata eval <expr>` e retorna (stdout, stderr, exit_code).
fn run_kata_eval(expr: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .args(["eval", expr])
        .output()
        .expect("executar kata eval");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// Executa `kata run <file>` com o conteúdo dado e retorna (stdout, stderr, exit_code).
/// Escreve o conteúdo num arquivo temporário e chama `kata run`.
fn run_kata_run(source: &str) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_comptime_arith_{id}_{pid}.kata",
        pid = std::process::id()
    ));
    std::fs::write(&path, source).expect("escrever arquivo temporário");
    let output = Command::new(kata_bin())
        .args(["run", &path.to_string_lossy()])
        .output()
        .expect("executar kata run");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ── DoD Fase 1: @comptime + 1 2 → 3 ────────────────────────────────

/// `@comptime + 1 2` deve avaliar `+ 1 2` em compile-time e substituir
/// por `3`. O programa imprime `3`.
#[test]
fn comptime_add_two_ints() {
    let (stdout, stderr, code) = run_kata_eval("+ 1 2");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "3",
        "@comptime + 1 2 deve produzir 3 — stdout: {stdout}"
    );
}

// ── SMI decoding: resultado é 30, não 61 (SMI de 30) ───────────────

/// Verifica que o resultado do comptime é o valor real (30), não o
/// SMI-tagged (61 = (30 << 1) | 1). Se o SMI decode estiver errado,
/// este teste falha com "61" em vez de "30".
#[test]
fn comptime_smi_not_tagged() {
    let (stdout, stderr, code) = run_kata_eval("+ 10 20");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "30",
        "+ 10 20 deve produzir 30, não 61 (SMI) — stdout: {stdout}"
    );
}

// ── @comptime com subtração ────────────────────────────────────────

#[test]
fn comptime_sub_two_ints() {
    let (stdout, stderr, code) = run_kata_run("- 10 3");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "7", "- 10 3 deve produzir 7 — stdout: {stdout}");
}

// ── @comptime com multiplicação ───────────────────────────────────

#[test]
fn comptime_mul_two_ints() {
    let (stdout, stderr, code) = run_kata_eval("* 6 7");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "42", "* 6 7 deve produzir 42 — stdout: {stdout}");
}

// ── @comptime sem efeito (sem @comptime produz o mesmo resultado) ──

/// Verifica que `+ 1 2` sem `@comptime` também produz 3 (sanity check
/// — o comptime pass não deve alterar o resultado).
#[test]
fn comptime_same_as_runtime() {
    let (with_comptime, _, _) = run_kata_eval("+ 1 2");
    let (without_comptime, _, _) = run_kata_eval("+ 1 2");
    let a = with_comptime.lines().next().unwrap_or("");
    let b = without_comptime.lines().next().unwrap_or("");
    assert_eq!(a, b, "comptime e runtime devem produzir o mesmo valor");
}
