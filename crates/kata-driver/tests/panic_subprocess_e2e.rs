//! Testes E2E de panic!/assert! via subprocess.
//!
//! Testes de abort (panic!/assert!(False)) não podem usar eval_src porque
//! panic! chama std::process::exit(1) que mata o runner de testes inteiro.
//! Estes testes executam `kata run` num processo filho isolado e verificam
//! o exit code e stderr.
//!
//! Os testes não-abortantes (assert!(True) retorna Unit) permanecem em
//! `kata-codegen/tests/panic_assert_e2e.rs` usando eval_src in-process.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Escreve source num arquivo temporário e executa `kata run`.
/// Retorna (stdout, stderr, exit_code).
fn run_kata(source: &str) -> (String, String, i32) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_panic_subprocess_{id}_{pid}.kata",
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

/// DoD 26: panic!("msg") aborta o processo com exit code != 0.
#[test]
fn panic_aborta_com_mensagem() {
    let src = "action crash => Unit\n    panic!(\"estado impossivel\")\ncrash!()";
    let (stdout, stderr, code) = run_kata(src);
    assert_ne!(
        code, 0,
        "panic! deve abortar (exit != 0) — code: {code}, stderr: {stderr}"
    );
    // stdout não deve ter output de echo! (panic aborta antes)
    assert!(
        stdout.is_empty() || !stdout.contains("ok"),
        "stdout não deve ter output normal — stdout: {stdout}"
    );
}

/// DoD 27: assert!(Boolean::False, "msg") aborta o processo.
/// O False desugara para panic!("msg").
#[test]
fn assert_false_aborta() {
    let src = "action valida => Unit\n    assert!(Boolean::False, \"x deve ser positivo\")\n    echo!(\"nao chega\")\nvalida!()";
    let (stdout, stderr, code) = run_kata(src);
    assert_ne!(
        code, 0,
        "assert!(False) deve abortar (exit != 0) — code: {code}, stderr: {stderr}"
    );
    // stdout não deve ter "nao chega" (assert aborta antes do echo!)
    assert!(
        !stdout.contains("nao chega"),
        "echo! não deve executar após assert!(False) — stdout: {stdout}"
    );
}

/// DoD 26: panic!("msg") imprime a mensagem no stderr antes de abortar.
#[test]
fn panic_imprime_mensagem_no_stderr() {
    let src = "action crash => Unit\n    panic!(\"estado impossivel\")\ncrash!()";
    let (_stdout, stderr, code) = run_kata(src);
    assert_ne!(code, 0, "deve abortar — code: {code}");
    assert!(
        stderr.contains("estado impossivel"),
        "stderr deve conter a mensagem do panic! — stderr: {stderr}"
    );
}

/// DoD 27: assert!(Boolean::False, "msg") imprime a mensagem no stderr.
#[test]
fn assert_false_imprime_mensagem_no_stderr() {
    let src =
        "action valida => Unit\n    assert!(Boolean::False, \"x deve ser positivo\")\nvalida!()";
    let (_stdout, stderr, code) = run_kata(src);
    assert_ne!(code, 0, "deve abortar — code: {code}");
    assert!(
        stderr.contains("x deve ser positivo"),
        "stderr deve conter a mensagem do assert! — stderr: {stderr}"
    );
}
