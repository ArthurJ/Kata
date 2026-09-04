//! F5: Rational na folha — match sobre tipos refined com Rational.
//!
//! Oráculos que testam exaustividade sobre refined types (Rational
//! com restrições de domínio), cobrindo domínio completo, fora do
//! domínio e otherwise.

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

/// RatUmOuDois: match sobre Rational refined com `rational 1:` / `rational 2:`.
/// F5: verde com output correto nos dois backends.
#[test]
fn rat_um_ou_dois_f5() {
    let path = write_temp_kata(
        "RatUmOuDois",
        r#"data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois

foo :: RatUmOuDois => Text
lambda n:
    match n
        rational 1: "um"
        rational 2: "dois"

action main
    match (RatUmOuDois (rational 1))
        Ok v: echo!(foo v)
        Err _: echo!("erro")
    match (RatUmOuDois (rational 2))
        Ok v: echo!(foo v)
        Err _: echo!("erro")

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "RatUmOuDois deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "um\ndois\n");
}

/// RatUmOuDois_zero: literal 0 fora do domínio — F5: TypeMismatch.
#[test]
fn rat_um_ou_dois_zero_f5() {
    let path = write_temp_kata(
        "RatUmOuDois_zero",
        r#"data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois

foo :: RatUmOuDois => Text
lambda n:
    match n
        rational 0: "zero"
        rational 1: "um"
        rational 2: "dois"

action main
    match (RatUmOuDois (rational 1))
        Ok v: echo!(foo v)
        Err _: echo!("erro")

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.mismatch"),
        "RatUmOuDois_zero deve dar TypeMismatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

/// RatNota: refined sobre Rational com otherwise, 4 braços.
/// F5.5: verde com output correto nos dois backends.
#[test]
fn rat_nota_with_otherwise_f5() {
    let path = write_temp_kata(
        "RatNota",
        r#"data (Rational, > _ (rational 0), < _ (rational 10)) as Nota

notaN :: Nota => Text
lambda n:
    match n
        rational 1: "ruim"
        rational 5: "médio"
        rational 9: "ótimo"
        otherwise: "outro"

action main
    match (Nota (rational 1))
        Ok v: echo!(notaN v)
        Err _: echo!("erro")
    match (Nota (rational 5))
        Ok v: echo!(notaN v)
        Err _: echo!("erro")
    match (Nota (rational 9))
        Ok v: echo!(notaN v)
        Err _: echo!("erro")
    match (Nota (rational 3))
        Ok v: echo!(notaN v)
        Err _: echo!("erro")

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "RatNota deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "ruim\nmédio\nótimo\noutro\n");
}
