//! Testes E2E — I/O proibido em `constant`.
//!
//! `constant` é determinístico: dado o mesmo código-fonte, produz o
//! mesmo TAST. I/O quebra essa propriedade. O comptime pass rejeita
//! `constant` cujo value contém ActionCall, Fork, Channel, etc. com
//! erro claro (`constant.io_forbidden`).
//!
//! Para embutir arquivos externos, usar `@embed_text{path: "..."}`
//! ou `@embed_bytes{path: "..."}` (PRD-embed-directive).

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn run_kata_run(source: &str) -> (String, String, i32) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_const_purity_{id}_{pid}.kata",
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

// ── Casos positivos: I/O rejeitado ──────────────────────────────

/// `constant x := echo!("hi")` — echo! é ActionCall, deve falhar
/// com `constant.io_forbidden`, não com parse error ou UnboundName.
#[test]
fn constant_io_echo_rejeitado() {
    // `echo!(x)` no top-level força o pipeline a passar pelo comptime.
    // Sem ele, `constant` sozinha não tem entry point e falha antes.
    let src = "constant x := echo!(\"hi\")\necho!(x)\n";
    let (_stdout, stderr, code) = run_kata_run(src);
    assert_ne!(code, 0, "constant com echo! deve falhar — code: {code}");
    assert!(
        stderr.contains("io_forbidden"),
        "stderr deve conter 'io_forbidden' — stderr: {stderr}"
    );
}

/// `constant x := sleep!(100)` — sleep! é ActionCall, deve falhar.
#[test]
fn constant_io_sleep_rejeitado() {
    let src = "constant x := sleep!(100)\necho!(x)\n";
    let (_stdout, stderr, code) = run_kata_run(src);
    assert_ne!(code, 0, "constant com sleep! deve falhar — code: {code}");
    assert!(
        stderr.contains("io_forbidden"),
        "stderr deve conter 'io_forbidden' — stderr: {stderr}"
    );
}

/// `constant x := echo!("hi")` deve mencionar @embed_text no help.
#[test]
fn constant_io_erro_menciona_embed() {
    let src = "constant x := echo!(\"hi\")\necho!(x)\n";
    let (_stdout, stderr, _code) = run_kata_run(src);
    assert!(
        stderr.contains("@embed_text"),
        "stderr deve sugerir @embed_text — stderr: {stderr}"
    );
}

// ── Caso negativo: constant pura continua funcionando ───────────

/// `constant x := 42` + `echo!(x)` — sem I/O, deve funcionar.
#[test]
fn constant_pura_continua_funcionando() {
    let src = "constant x := 42\necho!(x)\n";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "constant pura deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "42", "deve imprimir 42 — stdout: {stdout}");
}

/// `constant x := + 1 2` — função pura (Closure), deve funcionar.
#[test]
fn constant_com_chamada_pura_funciona() {
    let src = "constant x := + 1 2\necho!(x)\n";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "constant com + deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "3", "deve imprimir 3 — stdout: {stdout}");
}