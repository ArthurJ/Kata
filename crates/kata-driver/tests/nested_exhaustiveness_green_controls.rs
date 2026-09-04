//! Controles verdes (regressão) — exaustividade aninhada.
//!
//! Oráculos copiados de `tests/probe-nested/`. Cada teste executa `kata run`
//! (JIT) e verifica exit code + output/diagnóstico.

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

/// probeC: Some True + Some False + None — cobertura completa do payload.
#[test]
fn probe_c_completo_verde() {
    let path = write_temp_kata(
        "probeC",
        r#"foo :: Optional::(Boolean) => Text
lambda m:
    match m
        Some True: "tem true"
        Some False: "tem false"
        None: "nada"

action main
    echo!(foo (Some True))
    echo!(foo (Some False))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeC deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "tem true\ntem false\n");
}

/// probeD: cobertura completa + chamada só com Some True.
#[test]
fn probe_d_completo_verde() {
    let path = write_temp_kata(
        "probeD",
        r#"foo :: Optional::(Boolean) => Text
lambda m:
    match m
        Some True: "tem true"
        Some False: "tem false"
        None: "nada"

action main
    echo!(foo (Some True))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeD deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "tem true\n");
}

/// probeF2: wildcard sobre refined — controle verde.
#[test]
fn probe_f2_wildcard_refined_verde() {
    let path = write_temp_kata(
        "probeF2",
        r#"data (Int, > _ 0, < _ 3) as UmOuDois

foo :: UmOuDois => Text
lambda n:
    match n
        _ : "algum"

action main
    echo!(foo (1::UmOuDois))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeF2 deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "algum\n");
}

/// probeG: pattern aninhado qualificado + guards dentro da cláusula.
#[test]
fn probe_g_guards_intra_clausula_verde() {
    let path = write_temp_kata(
        "probeG",
        r#"foo :: Optional::(Int) => Text
lambda Optional::Some x:
    > x 0: "positivo"
    <= x 0: "zero ou negativo"
lambda Optional::None:
    "nada"

action main
    echo!(foo (Some 5))
    echo!(foo (Some (- 0 5)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeG deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "positivo\nzero ou negativo\n");
}

/// probeJ2: otherwise inútil pós-cobertura — isento (verde).
#[test]
fn probe_j2_otherwise_inutil_verde() {
    let path = write_temp_kata(
        "probeJ2",
        r#"foo :: Result::(Int, Text) => Text
lambda m:
    match m
        Ok v: "tem"
        Err _: "erro"
        otherwise: "impossivel"

action main
    echo!(foo (Ok 42))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeJ2 deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "tem\n");
}

/// probeK_deep: 3 níveis completo — regressão da matriz.
#[test]
fn probe_k_deep_completo_verde() {
    let path = write_temp_kata(
        "probeK_deep",
        r#"foo :: Optional::(Optional::(Boolean)) => Text
lambda m:
    match m
        Some Optional::Some True: "true dentro"
        Some Optional::Some False: "false dentro"
        Some Optional::None: "sem dentro"
        None: "nada"

action main
    echo!(foo (Some (Some True)))
    echo!(foo (Some (Some False)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeK_deep deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "true dentro\nfalse dentro\n");
}

/// probeK_grid: 2 params Boolean Boolean — grade 2×2 completa.
#[test]
fn probe_k_grid_completo_verde() {
    let path = write_temp_kata(
        "probeK_grid",
        r#"bar :: Boolean Boolean => Text
lambda True True: "vv"
lambda True False: "vf"
lambda False True: "fv"
lambda False False: "ff"

action main
    echo!(bar True True)
    echo!(bar False False)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeK_grid deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "vv\nff\n");
}

/// probeK_deep_paren: parêntese interno em braço — resolvido na Fase 0.
/// Controle verde: aninhamento com parênteses + cobertura completa.
#[test]
fn probe_k_deep_paren_verde() {
    let path = write_temp_kata(
        "probeK_deep_paren",
        r#"foo :: Optional::(Optional::(Boolean)) => Text
lambda m:
    match m
        Some (Some True): "true dentro"
        Some (Some False): "false dentro"
        Some None: "sem dentro"
        None: "nada"

action main
    echo!(foo (Some (Some True)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeK_deep_paren deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "true dentro\n");
}

/// RatUmOuDois_wildcard: wildcard sobre Rational refined — controle F5.
#[test]
fn rat_um_ou_dois_wildcard_verde() {
    let path = write_temp_kata(
        "RatUmOuDois_wildcard",
        r#"data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois

foo :: RatUmOuDois => Text
lambda n:
    match n
        _ : "algum"

action main
    match (RatUmOuDois (rational 1))
        Ok v: echo!(foo v)
        Err _: echo!("erro")

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(
        code, 0,
        "RatUmOuDois_wildcard deve exit 0 — stderr: {stderr}"
    );
    assert_eq!(stdout, "algum\n");
}
