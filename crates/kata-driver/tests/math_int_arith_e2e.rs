//! Testes E2E — aritmética Int (gcd, lcm, pow, signum) e sobrecargas Int→Float.

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

// ── Aritmética Int ──────────────────────────────────────────────

#[test]
fn math_gcd_lcm() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_gcd_lcm",
        r#"import complex
import math

action main => Unit
    echo!(gcd 12 8)
    echo!(lcm 4 6)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "4", "gcd 12 8 = 4");
    assert_eq!(lines[1], "12", "lcm 4 6 = 12");
}

#[test]
fn math_pow_signum() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_pow_signum",
        r#"import complex
import math

action main => Unit
    echo!(pow 2 10)
    echo!(signum (-5))
    echo!(signum 0)
    echo!(signum 42)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "1024", "pow 2 10 = 1024");
    assert_eq!(lines[1], "-1", "signum (-5) = -1");
    assert_eq!(lines[2], "0", "signum 0 = 0");
    assert_eq!(lines[3], "1", "signum 42 = 1");
}

// ── Sobrecargas Int → Float ────────────────────────────────────

#[test]
fn math_sin_cos_int() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sin_cos_int",
        r#"import complex
import math

action main => Unit
    echo!(sin 0)
    echo!(cos 0)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "0.0", "sin 0 = 0.0 (Int overload)");
    assert_eq!(lines[1], "1.0", "cos 0 = 1.0 (Int overload)");
}

#[test]
fn math_sqrt_exp_int() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sqrt_exp_int",
        r#"import complex
import math

action main => Unit
    echo!(sqrt 16)
    echo!(exp 0)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "4.0", "sqrt 16 = 4.0 (Int overload)");
    assert_eq!(lines[1], "1.0", "exp 0 = 1.0 (Int overload)");
}

#[test]
fn math_atan2_int() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_atan2_int",
        r#"import complex
import math

action main => Unit
    echo!(atan2 1 1)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let val: f64 = stdout.trim().parse().expect("atan2 1 1 deve ser Float");
    // atan2(1,1) = π/4 ≈ 0.7854
    assert!(
        (val - std::f64::consts::FRAC_PI_4).abs() < 1e-10,
        "atan2 1 1 = π/4: {val}"
    );
}
