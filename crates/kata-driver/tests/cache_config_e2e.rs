//! Testes E2E — `@cache` configuração (Fio 12, Fase 5).
//!
//! Estes testes exercitam a configuração do cache:
//! - capacity explícita
//! - @cache sem args (default LRU 256)
//! - @cache{} (dict vazio)
//! - capacity: 0 → erro de compilação

use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata run <file>` com o conteúdo dado e retorna (stdout, stderr, exit_code).
fn run_kata_run(source: &str) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_cache_config_e2e_{id}_{pid}.kata",
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

// ── capacity ──────────────────────────────────────────────────────

/// `@cache{capacity: 2}` (strategy default LRU). fib 10 com capacity 2:
/// eviction frequente, mas resultado correto.
#[test]
fn cache_capacity_explicit() {
    let src = "\
@cache{capacity: 2}
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

fib 10";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "55", "fib 10 deve ser 55 — stdout: {stdout}");
}

// ── @cache sem args ───────────────────────────────────────────────

/// `@cache` sozinho (sem dict) ativa LRU 256.
#[test]
fn cache_no_args() {
    let src = "\
@cache
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "10", "dobro 5 deve ser 10 — stdout: {stdout}");
}

/// `@cache{}` (dict vazio) ativa LRU 256.
#[test]
fn cache_empty_dict() {
    let src = "\
@cache{}
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "10", "dobro 5 deve ser 10 — stdout: {stdout}");
}

/// `@cache{capacity: 0}` → erro de compilação.
#[test]
fn cache_capacity_zero_error() {
    let src = "\
@cache{capacity: 0}
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (_stdout, stderr, code) = run_kata_run(src);
    assert_ne!(code, 0, "kata run deve falhar com capacity: 0");
    assert!(
        stderr.contains("capacidade de cache inválida") || stderr.contains("cache"),
        "stderr deve mencionar capacidade inválida — stderr: {stderr}"
    );
}
