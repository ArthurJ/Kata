//! F2: NonExhaustiveMatch — detecção de patterns não-exaustivos.
//!
//! Oráculos que devem falhar em compile-time com
//! `type.non_exhaustive_match` e witness apropriado.

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

/// probeA: Some(True) + None, SEM Some(False) — checker aceita por cegueira.
/// F2: NonExhaustiveMatch missing ["Some False"].
#[test]
fn probe_a_non_exhaustive() {
    let path = write_temp_kata(
        "probeA",
        r#"foo :: Optional::(Boolean) => Text
lambda m:
    match m
        Some True: "tem true"
        None: "nada"

action main
    echo!(foo (Some True))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeA deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert!(
        stderr.contains("Some (False)"),
        "witness deve ser Some (False) — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

/// probeB: chama foo com Some(False) — caso não coberto.
/// Hoje: SIGILL (exit -4/132). F2: NonExhaustiveMatch em compile-time.
#[test]
fn probe_b_non_exhaustive_compile() {
    let path = write_temp_kata(
        "probeB",
        r#"foo :: Optional::(Boolean) => Text
lambda m:
    match m
        Some True: "tem true"
        None: "nada"

action main
    echo!(foo (Some False))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    // F2: deve falhar em compile-time com NonExhaustiveMatch, NÃO SIGILL.
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeB deve dar NonExhaustiveMatch em compile — stderr: {stderr}"
    );
    assert_ne!(code, -4, "probeB não deve mais dar SIGILL");
}

/// probeM: match parcial sobre Result::(Int, Text) — Ok 0 + Err _.
/// F2: NonExhaustiveMatch missing ["Ok _"].
#[test]
fn probe_m_non_exhaustive() {
    let path = write_temp_kata(
        "probeM",
        r#"foo :: Result::(Int, Text) => Text
lambda m:
    match m
        Ok 0: "zero"
        Err _: "erro"

action main
    echo!(foo (Ok 0))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeM deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

/// probeK_deep_hole: 3 níveis com buraco — Some (Some False) removida.
/// F2: NonExhaustiveMatch com witness de 3 níveis ["Some (Some False)"].
#[test]
fn probe_k_deep_hole_non_exhaustive() {
    let path = write_temp_kata(
        "probeK_deep_hole",
        r#"foo :: Optional::(Optional::(Boolean)) => Text
lambda m:
    match m
        Some Optional::Some True: "true dentro"
        Some Optional::None: "sem dentro"
        None: "nada"

action main
    echo!(foo (Some (Some True)))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeK_deep_hole deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

/// probeK_grid_partial: grade 2×2 com 3 de 4 células.
/// Já RED hoje (non_exhaustive_match). F2: mesmo erro, com witness de matriz.
#[test]
fn probe_k_grid_partial_non_exhaustive() {
    let path = write_temp_kata(
        "probeK_grid_partial",
        r#"bar :: Boolean Boolean => Text
lambda True True: "vv"
lambda True False: "vf"
lambda False True: "fv"

action main
    echo!(bar True True)

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeK_grid_partial deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}
