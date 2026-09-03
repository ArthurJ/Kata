//! E2E — `show` de Instance de família polimórfica no JIT (A3g-adjacent).
//!
//! Responsabilidade: cravar que `echo!` (que despacha `show`) sobre
//! `Instance("NonZero", ...)` funciona no JIT. O monomorphizador não
//! fazia downcast de Instance para o tipo base (alias_of) ao resolver
//! `show`, deixando a Closure sem ffi_symbol e causando
//! `codegen.unsupported`.

use std::process::Command;

/// Roda `kata run <src>` (JIT) e retorna (stdout, stderr, code).
fn run_jit(path: &str) -> (String, String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kata"));
    cmd.args(["run", path]);
    let out = cmd.output().expect("kata run deve executar");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// Escreve source num .kata temporário de nome ÚNICO e retorna o path.
fn write_temp(name: &str, src: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kata_show_instance_e2e_{name}_{id}_{}.kata",
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().to_string()
}

/// `echo!` de `5 :: NonZero::Int` imprime 5 no JIT.
#[test]
fn show_nonzero_int_echo() {
    let source = r#"action main => Unit
    let z := 5 :: NonZero::Int
    echo!(z)
main!()"#;
    let path = write_temp("nzi", source);
    let (out, err, code) = run_jit(&path);
    assert_eq!(code, 0, "JIT deve exit 0 — stderr: {err}");
    assert_eq!(out, "5\n", "echo! de 5::NonZero::Int deve imprimir 5");
}

/// `show` explícito de `5 :: NonZero::Int` imprime 5 no JIT.
#[test]
fn show_nonzero_int_explicit() {
    let source = r#"action main => Unit
    let z := 5 :: NonZero::Int
    echo!(show z)
main!()"#;
    let path = write_temp("nzi_show", source);
    let (out, err, code) = run_jit(&path);
    assert_eq!(code, 0, "JIT deve exit 0 — stderr: {err}");
    assert_eq!(out, "5\n");
}

/// `echo!` de `3.14 :: NonZero::Float` imprime 3.14 no JIT.
#[test]
fn show_nonzero_float_echo() {
    let source = r#"action main => Unit
    let z := 3.14 :: NonZero::Float
    echo!(z)
main!()"#;
    let path = write_temp("nzf", source);
    let (out, err, code) = run_jit(&path);
    assert_eq!(code, 0, "JIT deve exit 0 — stderr: {err}");
    assert_eq!(out, "3.14\n");
}

/// `0 :: NonZero::Float` é rejeitado pelo JIT (predicado falha).
#[test]
fn zero_nonzero_float_rejeitado_jit() {
    let source = r#"action main => Unit
    let z := 0 :: NonZero::Float
    echo!(z)
main!()"#;
    let path = write_temp("nzf_zero", source);
    let (_out, _err, code) = run_jit(&path);
    assert_ne!(code, 0, "JIT deve rejeitar 0::NonZero::Float");
}

/// `echo!` de `5 :: NonZero::Int` com divisão continua funcionando.
#[test]
fn nonzero_int_div_funciona() {
    let source = r#"action main => Unit
    let z := 5 :: NonZero::Int
    echo!(/ 10 z)
main!()"#;
    let path = write_temp("nzi_div", source);
    let (out, err, code) = run_jit(&path);
    assert_eq!(code, 0, "JIT deve exit 0 — stderr: {err}");
    assert_eq!(out, "2\n");
}