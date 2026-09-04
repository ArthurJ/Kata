//! Testes E2E — sobrecargas Complex (trig, hyperbolic, raiz, log, norm, arg, div).

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

// ── Sobrecargas Complex → Complex ──────────────────────────────

#[test]
fn math_sin_cos_complex_real() {
    // sin/cos de complexo real (im=0) deve dar o mesmo que Float
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sin_cos_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (sin (Complex 0.0 0.0)))
    echo!(show (cos (Complex 0.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    // sin(0+0i) = 0+0i, cos(0+0i) = 1+0i
    assert!(lines[0].contains("0.0"), "sin(0+0i) re=0: {}", lines[0]);
    assert!(lines[1].contains("1.0"), "cos(0+0i) re=1: {}", lines[1]);
}

#[test]
fn math_exp_complex() {
    // exp(0+0i) = 1+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_exp_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (exp (Complex 0.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    assert!(out.contains("1.0"), "exp(0+0i) = 1+0i: {out}");
}

#[test]
fn math_sqrt_complex() {
    // sqrt(4+0i) = 2+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sqrt_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (sqrt (Complex 4.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    assert!(out.contains("2.0"), "sqrt(4+0i) = 2+0i: {out}");
}

#[test]
fn math_log_complex() {
    // log(1+0i) = 0+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_log_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (log (Complex 1.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    // log(1) = 0, mas em f64 ln(1) pode ter tiny epsilon
    assert!(
        out.contains("0.0") || out.contains("(-0.0"),
        "log(1+0i) ≈ 0+0i: {out}"
    );
}

#[test]
fn math_sinh_cosh_complex() {
    // sinh(0+0i) = 0+0i, cosh(0+0i) = 1+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sinh_cosh_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (sinh (Complex 0.0 0.0)))
    echo!(show (cosh (Complex 0.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert!(lines[0].contains("0.0"), "sinh(0+0i) = 0+0i: {}", lines[0]);
    assert!(lines[1].contains("1.0"), "cosh(0+0i) = 1+0i: {}", lines[1]);
}

#[test]
fn math_tan_complex() {
    // tan(0+0i) = 0+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_tan_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (tan (Complex 0.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    assert!(out.contains("0.0"), "tan(0+0i) = 0+0i: {out}");
}

#[test]
fn math_tanh_complex() {
    // tanh(0+0i) = 0+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_tanh_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (tanh (Complex 0.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    assert!(out.contains("0.0"), "tanh(0+0i) = 0+0i: {out}");
}

#[test]
fn math_asin_acos_complex() {
    // asin(0+0i) = 0+0i, acos(0+0i) = π/2+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_asin_acos_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (asin (Complex 0.0 0.0)))
    echo!(show (acos (Complex 0.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    // asin(0) = 0
    assert!(lines[0].contains("0.0"), "asin(0+0i) = 0+0i: {}", lines[0]);
    // acos(0) = π/2 ≈ 1.5708
    assert!(
        lines[1].contains("1.57") || lines[1].contains("1.5708"),
        "acos(0+0i) = π/2+0i: {}",
        lines[1]
    );
}

#[test]
fn math_atan_complex() {
    // atan(0+0i) = 0+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_atan_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (atan (Complex 0.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    assert!(out.contains("0.0"), "atan(0+0i) = 0+0i: {out}");
}

#[test]
fn math_asinh_acosh_atanh_complex() {
    // asinh(0+0i) = 0+0i, atanh(0+0i) = 0+0i
    // acosh(1+0i) = 0+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_hyper_inv_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (asinh (Complex 0.0 0.0)))
    echo!(show (acosh (Complex 1.0 0.0)))
    echo!(show (atanh (Complex 0.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert!(lines[0].contains("0.0"), "asinh(0+0i) = 0+0i: {}", lines[0]);
    assert!(lines[1].contains("0.0"), "acosh(1+0i) = 0+0i: {}", lines[1]);
    assert!(lines[2].contains("0.0"), "atanh(0+0i) = 0+0i: {}", lines[2]);
}

#[test]
fn math_cbrt_complex() {
    // cbrt(8+0i) = 2+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_cbrt_cx",
        r#"import complex
import math

action main => Unit
    echo!(show (cbrt (Complex 8.0 0.0)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    assert!(
        out.contains("1.999") || out.contains("2.0"),
        "cbrt(8+0i) ≈ 2+0i: {out}"
    );
}

// ── Sobrecargas Complex → Float (norm, arg) ────────────────────

#[test]
fn math_norm_complex() {
    // norm(3+4i) = 5
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_norm_cx",
        r#"import complex
import math

action main => Unit
    echo!(norm (Complex 3.0 4.0))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let val: f64 = stdout.trim().parse().expect("norm deve ser Float");
    assert!((val - 5.0).abs() < 1e-10, "norm(3+4i) = 5.0: {val}");
}

#[test]
fn math_arg_complex() {
    // arg(1+0i) = 0
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_arg_cx",
        r#"import complex
import math

action main => Unit
    echo!(arg (Complex 1.0 0.0))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let val: f64 = stdout.trim().parse().expect("arg deve ser Float");
    assert!(val.abs() < 1e-10, "arg(1+0i) = 0.0: {val}");
}

// ── Divisão Complex ────────────────────────────────────────────

#[test]
fn math_div_complex() {
    // (1+0i) / (1+0i) = 1+0i
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_div_cx",
        r#"import complex

action main => Unit
    echo!(show (div (Complex 1.0 0.0) (Complex 1.0 0.0) | Complex 0.0 0.0))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    assert!(out.contains("1.0"), "div(1+0i, 1+0i) = 1+0i: {out}");
}

#[test]
fn math_sin_complex_pure_imaginary() {
    // sin(0+pi/2 i) = i·sinh(pi/2) — parte real = 0, parte im > 0
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sin_pure_im",
        r#"import complex
import math

action main => Unit
    echo!(show (sin (Complex 0.0 (* pi 0.5))))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    let out = stdout.trim();
    // sin(iy) = i·sinh(y). Parte real ≈ 0, parte im = sinh(π/2) ≈ 2.301
    assert!(
        out.contains("2.30") || out.contains("2.301"),
        "sin(0 + π/2·i) = i·sinh(π/2) ≈ 2.301i: {out}"
    );
}
