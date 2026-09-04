//! F4: Refined na folha — match sobre tipos refined com literais.
//!
//! Oráculos que testam exaustividade sobre refined types (Int com
//! restrições de domínio), cobrindo domínio completo, fora do domínio
//! e parcial.

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

/// probeF: match sobre refined com literais cobrindo o domínio {1, 2}.
#[test]
fn probe_f_refined_folha() {
    let path = write_temp_kata(
        "probeF",
        r#"data (Int, > _ 0, < _ 3) as UmOuDois

foo :: UmOuDois => Text
lambda n:
    match n
        1: "um"
        2: "dois"

action main
    echo!(foo (1::UmOuDois))
    echo!(foo (2::UmOuDois))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeF deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "um\ndois\n");
}

/// probeF_fora_dominio: literal 0 fora do domínio {1, 2}.
#[test]
fn probe_f_fora_dominio() {
    let path = write_temp_kata(
        "probeF_fora_dominio",
        r#"data (Int, > _ 0, < _ 3) as UmOuDois

foo :: UmOuDois => Text
lambda n:
    match n
        1: "um"
        2: "dois"
        0: "zero"

action main
    echo!(foo (1::UmOuDois))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.mismatch"),
        "probeF_fora_dominio deve dar TypeMismatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

/// probeF_parcial: só `1:`, sem `2:` — buraco no domínio.
#[test]
fn probe_f_parcial() {
    let path = write_temp_kata(
        "probeF_parcial",
        r#"data (Int, > _ 0, < _ 3) as UmOuDois

foo :: UmOuDois => Text
lambda n:
    match n
        1: "um"

action main
    echo!(foo (1::UmOuDois))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeF_parcial deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}
