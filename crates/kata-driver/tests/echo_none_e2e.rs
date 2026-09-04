//! Testes E2E do A5: `echo!(None)` e `show` de variante unitária sem tipo concreto.
//!
//! Bug A5: `echo!(None)` no JIT rejeitava com `codegen.unsupported` porque
//! `show` de `Generic("Optional", [Var("T")])` não era instanciado pelo
//! monomorphizador (guard bloqueava `T = Var`). Fix: quando `show` tem
//! type params não-resolvidos, instanciar com `T = Unit` como fallback.
//! O braço `None` (TextLit) não precisa de T; o braço `Some` nunca executa
//! para `None`.
//!
//! Bug A3c: `show Optional::None` falhava com `ffi_not_found` pelo mesmo
//! root cause — também resolvido pelo fix.

use std::process::Command;

/// Caminho para o binário `kata` compilado em debug.
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata run <src>` e retorna (exit_code, stdout, stderr).
fn run_kata(src: &str) -> (i32, String, String) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let bin = kata_bin();
    let tmp = std::env::temp_dir().join(format!(
        "kata_a5_{}_{}.kata",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, src).expect("escrever arquivo temporário");

    let output = Command::new(&bin)
        .arg("run")
        .arg(&tmp)
        .output()
        .expect("executar kata run");

    let _ = std::fs::remove_file(&tmp);

    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// `echo!(None)` → imprime "None", exit 0.
#[test]
fn echo_none_imprime_none() {
    let (code, stdout, _stderr) = run_kata("echo!(None)");
    assert_eq!(code, 0, "echo!(None) deve ter exit 0, teve {code}");
    assert_eq!(stdout.trim(), "None", "stdout deve ser 'None'");
}

/// `echo!(show None)` → imprime "None", exit 0.
#[test]
fn show_none_imprime_none() {
    let (code, stdout, _stderr) = run_kata("echo!(show None)");
    assert_eq!(code, 0, "echo!(show None) deve ter exit 0");
    assert_eq!(stdout.trim(), "None");
}

/// `echo!(show Optional::None)` → imprime "None", exit 0 (A3c).
#[test]
fn show_qualified_none_imprime_none() {
    let (code, stdout, _stderr) = run_kata("echo!(show Optional::None)");
    assert_eq!(code, 0, "echo!(show Optional::None) deve ter exit 0");
    assert_eq!(stdout.trim(), "None");
}

/// `echo!(show (Some 42))` — não deve regredir. Imprime "Some(42)".
#[test]
fn show_some_nao_regride() {
    let (code, stdout, _stderr) = run_kata("echo!(show (Some 42))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "Some(42)");
}

/// `echo!(show (Ok 42))` — não deve regredir. Imprime "Ok(42)".
#[test]
fn show_ok_nao_regride() {
    let (code, stdout, _stderr) = run_kata("echo!(show (Ok 42))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "Ok(42)");
}

/// `echo!(show (Err "fail"))` — não deve regredir. Imprime `Err("fail")`.
#[test]
fn show_err_nao_regride() {
    let (code, stdout, _stderr) = run_kata("echo!(show (Err \"fail\"))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "Err(\"fail\")");
}

/// `echo!(None)` no interpretador também funciona.
#[test]
fn echo_none_interp_imprime_none() {
    let bin = kata_bin();
    let tmp = std::env::temp_dir().join(format!("kata_a5_interp_{}.kata", std::process::id()));
    std::fs::write(&tmp, "echo!(None)").expect("escrever arquivo temporário");

    let output = Command::new(&bin)
        .arg("run")
        .arg("--interp")
        .arg(&tmp)
        .output()
        .expect("executar kata run --interp");

    let _ = std::fs::remove_file(&tmp);

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert_eq!(code, 0, "echo!(None) --interp deve ter exit 0");
    assert_eq!(stdout.trim(), "None");
}
