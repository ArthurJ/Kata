//! Testes E2E — constantes matemáticas (pi, euler, phi).

use std::fs;
use std::process::Command;

fn kata_bin() -> String {
    std::env::var("KATA_BIN").unwrap_or_else(|_| {
        let manifest = env!("CARGO_MANIFEST_DIR");
        format!("{manifest}/../../target/debug/kata")
    })
}

/// Cria um arquivo `.kata` temporário e retorna o path.
fn write_temp_kata(dir: &std::path::Path, name: &str, content: &str) -> String {
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata run <path>` e retorna (stdout, stderr, exit_code).
fn run_kata(path: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .arg("run")
        .arg(path)
        .output()
        .expect("executar kata run");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ── Constantes ──────────────────────────────────────────────────

#[test]
fn math_constant_pi() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_pi",
        r#"import complex
import math

echo!(pi)"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.trim().contains("3.141592653589793"),
        "esperava pi. stdout: {stdout}"
    );
}

#[test]
fn math_constant_euler() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_euler",
        r#"import complex
import math

echo!(euler)"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.trim().contains("2.718281828459045"),
        "esperava euler. stdout: {stdout}"
    );
}

#[test]
fn math_constant_phi() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_phi",
        r#"import complex
import math

echo!(phi)"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.trim().contains("1.618033988749895"),
        "esperava phi. stdout: {stdout}"
    );
}

// ── Constant + função juntas ────────────────────────────────────

#[test]
fn math_constant_e_funcao_juntas() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_const_fn",
        r#"import complex
import math

action main => Unit
    echo!(pi)
    echo!(sin pi)
    echo!(sqrt (* pi pi))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert!(lines[0].contains("3.14159"), "pi: {}", lines[0]);
    // sin(pi) ≈ 0 (muito próximo de zero)
    assert!(
        lines[1].parse::<f64>().unwrap_or(1.0).abs() < 1e-10,
        "sin(pi) ≈ 0: {}",
        lines[1]
    );
    // sqrt(pi²) = pi
    assert!(
        lines[2].parse::<f64>().unwrap_or(0.0).abs() - std::f64::consts::PI < 1e-10,
        "sqrt(pi²) = pi: {}",
        lines[2]
    );
}
