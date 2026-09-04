//! Testes E2E — funções matemáticas Float (trig, raiz, floor/ceil, min/max).

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

// ── Trigonométricas ─────────────────────────────────────────────

#[test]
fn math_sin_cos() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sin_cos",
        r#"import complex
import math

action main => Unit
    echo!(sin 0.0)
    echo!(cos 0.0)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "0.0", "sin 0.0 = 0.0");
    assert_eq!(lines[1], "1.0", "cos 0.0 = 1.0");
}

#[test]
fn math_sqrt_cbrt() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sqrt_cbrt",
        r#"import complex
import math

action main => Unit
    echo!(sqrt 16.0)
    echo!(cbrt 27.0)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "4.0", "sqrt 16.0 = 4.0");
    assert_eq!(lines[1], "3.0", "cbrt 27.0 = 3.0");
}

// ── Floor e ceil ────────────────────────────────────────────────

#[test]
fn math_floor_ceil() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_floor_ceil",
        r#"import complex
import math

action main => Unit
    echo!(floor 3.7)
    echo!(ceil 3.2)
    echo!(floor (-3.7))
    echo!(ceil (-3.2))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "3", "floor 3.7 = 3");
    assert_eq!(lines[1], "4", "ceil 3.2 = 4");
    assert_eq!(lines[2], "-4", "floor (-3.7) = -4");
    assert_eq!(lines[3], "-3", "ceil (-3.2) = -3");
}

// ── Min/max genéricos (do core.kata) ────────────────────────────

#[test]
fn math_min_max_int() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_min_max_int",
        r#"action main => Unit
    echo!(min 3 7)
    echo!(max 3 7)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "3", "min 3 7 = 3");
    assert_eq!(lines[1], "7", "max 3 7 = 7");
}

#[test]
fn math_min_max_float() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_min_max_float",
        r#"action main => Unit
    echo!(min 3.5 2.1)
    echo!(max 3.5 2.1)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "2.1", "min 3.5 2.1 = 2.1");
    assert_eq!(lines[1], "3.5", "max 3.5 2.1 = 3.5");
}
