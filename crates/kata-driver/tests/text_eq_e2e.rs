//! E2E — `Text implements EQ` (igualdade de texto por conteúdo).
//!
//! Responsabilidade: cravar que `=`/`!=` sobre Text compila e compara
//! por CONTEÚDO (C string), em ambos os backends. EQ cobre refined sobre
//! Text (predicado `= _ _`) — sem isso, nenhum refined de Text é
//! declarável (probe 2026-08-30: `= _ _` sobre Text = no_overload).

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
        "kata_text_eq_e2e_{name}_{id}_{}.kata",
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

/// `=` sobre Text: iguais e diferentes.
#[test]
fn eq_text_iguais_e_diferentes() {
    let source = r#"action main => Unit
    match (= "ola" "ola")
        Boolean::True: echo!("iguais")
        Boolean::False: echo!("diferentes")
    match (= "ola" "mundo")
        Boolean::True: echo!("iguais")
        Boolean::False: echo!("diferentes")
main!()"#;
    assert_both(&source, "iguais\ndiferentes\n");
}

/// `!=` sobre Text (o outro método de EQ).
#[test]
fn neq_text() {
    let source = r#"action main => Unit
    match (!= "ola" "ola")
        Boolean::True: echo!("diferentes")
        Boolean::False: echo!("iguais")
    match (!= "ola" "mundo")
        Boolean::True: echo!("diferentes")
        Boolean::False: echo!("iguais")
main!()"#;
    assert_both(&source, "iguais\ndiferentes\n");
}

/// Text com interpolação: dois literals com mesmo conteúdo são iguais
/// mesmo se construídos separadamente (conteúdo, não ponteiro).
#[test]
fn eq_text_completa_conteudo() {
    let source = r#"action main => Unit
    let a := "kata"
    let b := "ka"
    let c := "ta"
    match (= a (+ b c))
        Boolean::True: echo!("conteudo_igual")
        Boolean::False: echo!("conteudo_diferente")
main!()"#;
    assert_both(&source, "conteudo_igual\n");
}
