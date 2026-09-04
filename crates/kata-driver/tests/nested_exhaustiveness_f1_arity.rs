//! F1: ArityMismatch e falso-positivo de redundância — exaustividade aninhada.
//!
//! Pré-F1: panic 101 (helpers.rs:104, index out of bounds). Com parser
//! aridade-consciente, não há mais panic. F2: verde, sem falso-positivo.

use std::fs;
use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e-nested");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

fn run_kata(args: &[&str]) -> (String, String, i32) {
    let result = Command::new(kata_bin())
        .args(args)
        .output()
        .expect("executar kata");
    (
        String::from_utf8_lossy(&result.stdout).to_string(),
        String::from_utf8_lossy(&result.stderr).to_string(),
        result.status.code().unwrap_or(-1),
    )
}

fn run_kata_file(path: &str) -> (String, String, i32) {
    run_kata(&["run", path])
}

/// probeE: `lambda Some True:` em função de 1 param.
/// Pré-F1: panic 101 (helpers.rs:104, index out of bounds) — parseava como
/// 2 patterns. Com parser aridade-consciente, parseia como 1 pattern
/// Variant{Some, [True]}. Não há mais panic.
/// F1: falso-positivo `type.redundant_clause` (mesmo bug de probeE2).
/// F2: verde, sem falso-positivo — motor Maranget desce payloads,
/// Some True e Some False não são redundantes.
#[test]
fn probe_e_nao_panic() {
    let path = write_temp_kata(
        "probeE",
        r#"foo :: Optional::(Boolean) => Text
lambda Some True: "tem true"
lambda Some False: "tem false"
lambda None: "nada"

action main
    echo!(foo (Some True))
    echo!(foo (Some False))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);

    // NÃO pode panicar (exit 101).
    assert_ne!(code, 101, "probeE não deve panicar — stderr: {stderr}");
    // F2: verde — motor reconhece Some True / Some False como não-redundantes.
    assert_eq!(code, 0, "probeE deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "tem true\ntem false\n");
}

/// probeK_arity_tuple: `lambda True True:` em função de 1 param tupla.
/// Pré-F1: panic 101 (helpers.rs:104, index out of bounds) — parseava como
/// 2 patterns. Com parser aridade-consciente (arity=1, tupla é 1 param),
/// parseia como 1 pattern Variant{True, [True]} — typeck rejeita gracioso
/// (`True` não é variante de `(Boolean, Boolean)`).
/// F1: erro gracioso (não panic). O bound-check cobre casos onde
/// o parser produz N≠arity patterns (ex: lambda anônimo).
#[test]
fn probe_k_arity_tuple_nao_panic() {
    let path = write_temp_kata(
        "probeK_arity_tuple",
        r#"bar :: (Boolean, Boolean) => Text
lambda True True: "vv"
lambda True False: "vf"

action main
    echo!(bar (True, True))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);

    assert_ne!(
        code, 101,
        "probeK_arity_tuple não deve panicar — stderr: {stderr}"
    );
    assert_ne!(
        code, 0,
        "probeK_arity_tuple não deve ser verde — stderr: {stderr}"
    );
}

/// probeE2: cláusulas lambda com variant qualificado aninhado.
/// F1: falso-positivo `type.redundant_clause` (a 2ª cláusula Some False
/// era rejeitada como redundante). F2: verde, sem falso-positivo —
/// motor Maranget desce payloads e reconhece Some True / Some False
/// como não-redundantes.
#[test]
fn probe_e2_nao_panic_redundancia() {
    let path = write_temp_kata(
        "probeE2",
        r#"foo :: Optional::(Boolean) => Text
lambda Optional::Some True: "tem true"
lambda Optional::Some False: "tem false"
lambda Optional::None: "nada"

action main
    echo!(foo (Some True))
    echo!(foo (Some False))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);

    // F2: verde — motor reconhece Some True / Some False como não-redundantes.
    assert_ne!(code, 101, "probeE2 não deve panicar — stderr: {stderr}");
    assert_eq!(code, 0, "probeE2 deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "tem true\ntem false\n");
}
