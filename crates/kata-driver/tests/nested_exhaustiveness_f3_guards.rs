//! F3: Guards entre cláusulas — patterns repetidos com guards distintos.
//!
//! Oráculos que testam guards espalhados por cláusulas com o MESMO
//! pattern base, incluindo a sintaxe `with`.

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

/// probeH: guards espalhados por cláusulas com o MESMO pattern.
/// F3: verde com output correto.
#[test]
fn probe_h_guards_entre_clausulas() {
    let path = write_temp_kata(
        "probeH",
        r#"foo :: Optional::(Int) => Text
lambda Optional::Some x:
    > x 0: "positivo"
lambda Optional::Some x:
    <= x 0: "zero ou negativo"
lambda Optional::None:
    "nada"

action main
    echo!(foo (Some 5))
    echo!(foo (Some (- 0 5)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeH deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "positivo\nzero ou negativo\n");
}

/// probeH_with: igual ao probeH, mas guards via `with`.
#[test]
fn probe_h_with_guards_entre_clausulas() {
    let path = write_temp_kata(
        "probeH_with",
        r#"foo :: Optional::(Int) => Text
lambda Optional::Some x:
    positivo: "positivo"
    with
        positivo := > x 0
lambda Optional::Some x:
    nao_positivo: "zero ou negativo"
    with
        nao_positivo := <= x 0
lambda Optional::None:
    "nada"

action main
    echo!(foo (Some 5))
    echo!(foo (Some (- 0 5)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeH_with deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "positivo\nzero ou negativo\n");
}
