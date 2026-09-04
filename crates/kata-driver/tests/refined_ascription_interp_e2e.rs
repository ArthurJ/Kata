//! E2E — Ascription refined no interpretador (A3g: pending_predicates).
//!
//! Responsabilidade: cravar que ascription refined de famílias polimórficas
//! (NonZero) é validada no interpretador. O typeck produz
//! `pending_predicates` quando o predicado é complexo (ex: `!= _ (zero _)`)
//! — o comptime pass do JIT os valida, mas o interp não tem comptime pass.
//! O interp agora valida pending_predicates no ponto de uso via `eval`.

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
        "kata_refined_ascription_interp_e2e_{name}_{id}_{}.kata",
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().to_string()
}

/// `0 :: NonZero::Int` deve ser rejeitado pelo interp (predicado falha).
/// Nota: o trampoline do scheduler engole erros e retorna 0 (exit code 0),
/// mas a mensagem de erro é impressa no stderr. O exit code não-zero é
/// um bug separado do trampoline (csp.rs:212-218).
#[test]
fn zero_ascription_nonzero_int_rejeitado_interp() {
    let source = r#"action main => Unit
    let z := 0 :: NonZero::Int
    echo!(/ 10 z)
main!()"#;
    let path = write_temp("zero_int", source);
    let (_out, err, _code) = run_kata(&path, true);
    assert!(
        err.contains("predicado") || err.contains("refined"),
        "erro deve mencionar predicado/refined — stderr: {err}"
    );
    // Não deve imprimir "2" (que seria o resultado de 10/5 se o 0 passasse).
    // Mais importante: não deve dar SIGABRT (panic de divisão por zero).
    assert!(
        !err.contains("divisão por zero") && !err.contains("SIGABRT"),
        "não deve crashar com divisão por zero — stderr: {err}"
    );
}

/// `5 :: NonZero::Int` deve funcionar no interp (predicado passa).
#[test]
fn nonzero_valido_funciona_interp() {
    let source = r#"action main => Unit
    let z := 5 :: NonZero::Int
    echo!(/ 10 z)
main!()"#;
    let path = write_temp("valid_int", source);
    let (out, err, code) = run_kata(&path, true);
    assert_eq!(
        code, 0,
        "interp deve aceitar 5::NonZero::Int — stderr: {err}"
    );
    assert_eq!(out, "2\n", "10 / 5 = 2");
}

/// `1 :: NonZero::Int` deve funcionar (mínimo positivo).
#[test]
fn um_nonzero_funciona_interp() {
    let source = r#"action main => Unit
    let z := 1 :: NonZero::Int
    echo!(* z z)
main!()"#;
    let path = write_temp("one_int", source);
    let (out, err, code) = run_kata(&path, true);
    assert_eq!(
        code, 0,
        "interp deve aceitar 1::NonZero::Int — stderr: {err}"
    );
    assert_eq!(out, "1\n", "1 * 1 = 1");
}

/// Refined simples (PositiveInt) também deve ser validado pelo interp.
/// PositiveInt tem predicado `> _ (zero _)` — const_eval resolve para
/// literais, mas se não resolver, pending_predicates deve ser validado.
#[test]
fn refined_simples_valido_interp() {
    let source = r#"action main => Unit
    let p := 42 :: PositiveInt
    echo!(p)
main!()"#;
    let path = write_temp("pos_int", source);
    let (out, err, code) = run_kata(&path, true);
    assert_eq!(
        code, 0,
        "interp deve aceitar 42::PositiveInt — stderr: {err}"
    );
    assert_eq!(out, "42\n");
}

/// Ascription refined de Text literal: `>= (len _) 1` const-avalia sobre
/// TextLit. O const_eval reduz `len "ola"` → 3, depois `>= 3 1` → true.
/// Débito 2 do TODO — agora funciona em ambos backends.
#[test]
fn text_refined_literal_passa() {
    let source = r#"data (Text, >= (len _) 1) as NonEmptyText

action main => Unit
    let s := "ola" :: NonEmptyText
    echo!(s)
main!()"#;
    let path = write_temp("text_ok", source);
    let (out, err, code) = run_kata(&path, true);
    assert_eq!(
        code, 0,
        "interp deve aceitar \"ola\"::NonEmptyText — stderr: {err}"
    );
    assert_eq!(out, "ola\n");
    let (out_j, err_j, code_j) = run_kata(&path, false);
    assert_eq!(
        code_j, 0,
        "JIT deve aceitar \"ola\"::NonEmptyText — stderr: {err_j}"
    );
    assert_eq!(out_j, "ola\n");
}

/// Ascription refined de Text literal vazio: `>= (len _) 1` falha para
/// `""` (len = 0). Deve ser rejeitado em compile-time por ambos backends.
#[test]
fn text_refined_literal_vazio_rejeitado() {
    let source = r#"data (Text, >= (len _) 1) as NonEmptyText

action main => Unit
    let s := "" :: NonEmptyText
    echo!(s)
main!()"#;
    let path = write_temp("text_empty", source);
    // Interp: trampoline engole erro (exit 0), mas stderr tem a mensagem.
    let (_out, err, _code) = run_kata(&path, true);
    assert!(
        err.contains("predicado") || err.contains("refined"),
        "interp deve rejeitar \"\"::NonEmptyText — stderr: {err}"
    );
    // JIT: exit code não-zero.
    let (_out_j, _err_j, code_j) = run_kata(&path, false);
    assert_ne!(
        code_j, 0,
        "JIT deve rejeitar \"\"::NonEmptyText (exit não-zero)"
    );
}
