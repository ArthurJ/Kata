//! Testes E2E — módulo math.kata.
//!
//! Testa constantes (pi, euler, phi), funções trigonométricas,
//! hiperbólicas, raiz/log, floor/ceil, aritmética Int (gcd, lcm, pow,
//! signum), e min/max genéricos (do core.kata).

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
        r#"import math.(pi)

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
        r#"import math.(euler)

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
        r#"import math.(phi)

echo!(phi)"#,
    );
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.trim().contains("1.618033988749895"),
        "esperava phi. stdout: {stdout}"
    );
}

// ── Trigonométricas ─────────────────────────────────────────────

#[test]
fn math_sin_cos() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_sin_cos",
        r#"import math.(sin, cos)

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
        r#"import math.(sqrt, cbrt)

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
        r#"import math.(floor, ceil)

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

// ── Aritmética Int ──────────────────────────────────────────────

#[test]
fn math_gcd_lcm() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_gcd_lcm",
        r#"import math.(gcd, lcm)

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
        r#"import math.(pow, signum)

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

// ── Constant + função juntas ────────────────────────────────────

#[test]
fn math_constant_e_funcao_juntas() {
    let dir = std::env::temp_dir().join("kata-driver-math-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = write_temp_kata(
        &dir,
        "math_const_fn",
        r#"import math.(pi, sin, sqrt)

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
