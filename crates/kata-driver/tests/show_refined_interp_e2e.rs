//! E2E — show de tipos refined no interpretador.
//!
//! Responsabilidade: cravar que `echo!` de um valor refined mostra o
//! VALOR DO TIPO BASE (mesma regra da síntese `__kata_show__{Refined}`
//! do codegen: refined delega ao show do base). O interp interceptava
//! `__kata_show__*` e formatava como struct vazio (`PositiveInt()`).

use std::process::Command;

/// Roda `kata run [--interp] <src>` e retorna (stdout, stderr, code).
fn run_kata(path: &str, interp: bool) -> (String, String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kata"));
    if interp {
        cmd.args(["run", "--interp", path]);
    } else {
        cmd.args(["run", path]);
    }
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
        "kata_show_refined_e2e_{name}_{id}_{}.kata",
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().to_string()
}

/// Ambos os backends devem ter exit 0 e o mesmo stdout.
fn assert_both(src: &str, expected: &str) {
    let path = write_temp("case", src);
    let (out_i, err_i, code_i) = run_kata(&path, true);
    assert_eq!(
        code_i, 0,
        "interp deve exit 0 — stderr: {err_i}\nstdout: {out_i}"
    );
    assert_eq!(out_i, expected, "interp: stdout divergente");
    let (out_j, err_j, code_j) = run_kata(&path, false);
    assert_eq!(
        code_j, 0,
        "JIT deve exit 0 — stderr: {err_j}\nstdout: {out_j}"
    );
    assert_eq!(out_j, expected, "JIT: stdout divergente");
}

/// Refined sobre Int: `echo!(x)` mostra `5`, não `PositiveInt()`.
#[test]
fn show_refined_int_delega_ao_base() {
    let source = r#"data (Int, > _ 0) as PositiveInt

action main => Unit
    let x := 5::PositiveInt
    echo!(x)
main!()"#;
    assert_both(&source, "5\n");
}

/// Refined sobre Int com dois predicados (PositiveInt e Percentage no
/// mesmo módulo) — cada um delega ao próprio base.
#[test]
fn show_refined_multiplos_tipos() {
    let source = r#"data (Int, > _ 0) as PositiveInt
data (Int, > _ 0, < _ 100) as Percentage

action main => Unit
    let x := 5::PositiveInt
    let p := 50::Percentage
    echo!(x)
    echo!(p)
main!()"#;
    assert_both(&source, "5\n50\n");
}

/// Refined sobre Text: delega ao show de Text (aspas). Via construtor
/// (a const-eval de ascription refined cobre literals numéricos, não Text).
#[test]
fn show_refined_text_delega_ao_base() {
    let source = r#"data (Text, = _ _) as NonEmpty

action main => Unit
    match (NonEmpty "ola")
        Ok s: echo!(s)
        Err _: echo!("erro")
main!()"#;
    assert_both(&source, "ola\n");
}

/// Refined sobre Float: delega ao show de Float.
#[test]
fn show_refined_float_delega_ao_base() {
    let source = r#"data (Float, > _ 0.0) as PosFloat

action main => Unit
    let f := 2.5::PosFloat
    echo!(f)
main!()"#;
    assert_both(&source, "2.5\n");
}
