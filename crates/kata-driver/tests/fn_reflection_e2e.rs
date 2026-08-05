//! Testes E2E — Reflexão de funções via DotAccess.
//!
//! PRD-fn-reflection.md: `f.name`, `f.arity`, `f.param_types`,
//! `f.return_type`, `f.is_action` — metadata estática de funções
//! e actions via DotAccess.

use std::fs;
use std::process::Command;
use std::sync::Once;

static SETUP: Once = Once::new();

fn ensure_kata_rt_built() {
    SETUP.call_once(|| {
        let build_root = env!("KATA_BUILD_ROOT");
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let target_dir = std::path::Path::new(build_root)
            .join("target")
            .join(profile);
        let static_lib = target_dir.join("libkata_rt.a");
        let dynamic_lib = target_dir.join("libkata_rt.so");

        if static_lib.exists() && dynamic_lib.exists() {
            return;
        }

        let mut cmd = Command::new("cargo");
        cmd.current_dir(build_root)
            .arg("build")
            .arg("-p")
            .arg("kata-rt");

        if profile == "release" {
            cmd.arg("--release");
        }

        let status = cmd
            .status()
            .expect("setup: não foi possível executar `cargo build -p kata-rt`");

        assert!(
            status.success(),
            "setup: `cargo build -p kata-rt` falhou — \
             libkata_rt.a é necessária para AOT"
        );
    });
}

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e-reflection");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

fn run_kata_build(input: &str, output: &str) -> (String, i32) {
    ensure_kata_rt_built();
    let output_path = std::env::temp_dir()
        .join("kata-driver-e2e-reflection")
        .join(output);
    let output_str = output_path.to_string_lossy().to_string();

    let result = Command::new(kata_bin())
        .args(["build", input, "-o", &output_str])
        .output()
        .expect("executar kata build");

    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    let stderr = String::from_utf8_lossy(&result.stderr).to_string();
    let combined = if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}{stderr}")
    };
    (combined, result.status.code().unwrap_or(-1))
}

fn run_built_binary(name: &str) -> (String, i32) {
    let bin_path = std::env::temp_dir()
        .join("kata-driver-e2e-reflection")
        .join(name);
    let result = Command::new(&bin_path)
        .output()
        .expect("executar binário AOT");
    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    (stdout, result.status.code().unwrap_or(-1))
}

/// Helper: build + run + retornar primeira linha do stdout.
fn build_and_get_first_line(name: &str, src: &str) -> String {
    let path = write_temp_kata(name, src);
    let (build_out, build_code) = run_kata_build(&path, &format!("{name}_bin"));
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary(&format!("{name}_bin"));
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");

    stdout.lines().next().unwrap_or("").to_string()
}

/// Helper: build deve falhar (erro de compilação).
fn build_and_expect_error(name: &str, src: &str) -> String {
    let path = write_temp_kata(name, src);
    let (output, code) = run_kata_build(&path, &format!("{name}_bin"));
    assert_ne!(
        code, 0,
        "kata build deve falhar (erro esperado) — output: {output}"
    );
    output
}

// ── DoD 1: f.name em função pura (estático) ───────────────────────

#[test]
fn fn_reflection_static_name() {
    let first = build_and_get_first_line(
        "fn_reflection_static_name",
        "soma :: Int Int => Int\nlambda a b: + a b\naction main => Unit\n    echo!(soma.name)\nmain!()",
    );
    assert_eq!(first, "soma", "soma.name deve imprimir \"soma\"");
}

// ── DoD 2: f.arity em função pura (estático) ──────────────────────

#[test]
fn fn_reflection_static_arity() {
    let first = build_and_get_first_line(
        "fn_reflection_static_arity",
        "soma :: Int Int => Int\nlambda a b: + a b\naction main => Unit\n    echo!(soma.arity)\nmain!()",
    );
    assert_eq!(first, "2", "soma.arity deve imprimir \"2\"");
}

// ── DoD 3: f.return_type em função pura (estático) ─────────────────

#[test]
fn fn_reflection_static_return_type() {
    let first = build_and_get_first_line(
        "fn_reflection_static_return_type",
        "soma :: Int Int => Int\nlambda a b: + a b\naction main => Unit\n    echo!(soma.return_type)\nmain!()",
    );
    assert_eq!(first, "Int", "soma.return_type deve imprimir \"Int\"");
}

// ── DoD 7: Action reflection (estático via DispatchTable) ──────────

#[test]
fn fn_reflection_action_name() {
    let first = build_and_get_first_line(
        "fn_reflection_action_name",
        "action processar(x::Int) => Int\n    + x 1\naction main => Unit\n    echo!(processar.name)\nmain!()",
    );
    assert_eq!(
        first, "processar",
        "processar.name deve imprimir \"processar\""
    );
}

// ── DoD 8: Action is_action → True ────────────────────────────────

#[test]
fn fn_reflection_action_is_action() {
    let first = build_and_get_first_line(
        "fn_reflection_action_is_action",
        "action processar(x::Int) => Int\n    + x 1\naction main => Unit\n    echo!(processar.is_action)\nmain!()",
    );
    assert_eq!(first, "True", "processar.is_action deve imprimir \"True\"");
}

// ── DoD 9: Function is_action → False ─────────────────────────────

#[test]
fn fn_reflection_function_is_action() {
    let first = build_and_get_first_line(
        "fn_reflection_function_is_action",
        "soma :: Int Int => Int\nlambda a b: + a b\naction main => Unit\n    echo!(soma.is_action)\nmain!()",
    );
    assert_eq!(first, "False", "soma.is_action deve imprimir \"False\"");
}

// ── DoD 10: Action arity (estático) ───────────────────────────────

#[test]
fn fn_reflection_action_arity() {
    let first = build_and_get_first_line(
        "fn_reflection_action_arity",
        "action processar(x::Int) => Int\n    + x 1\naction main => Unit\n    echo!(processar.arity)\nmain!()",
    );
    assert_eq!(first, "1", "processar.arity deve imprimir \"1\"");
}

// ── DoD 11: Action return_type (estático) ─────────────────────────

#[test]
fn fn_reflection_action_return_type() {
    let first = build_and_get_first_line(
        "fn_reflection_action_return_type",
        "action processar(x::Int) => Int\n    + x 1\naction main => Unit\n    echo!(processar.return_type)\nmain!()",
    );
    assert_eq!(first, "Int", "processar.return_type deve imprimir \"Int\"");
}

// ── DoD 14: f.foo (field desconhecido) deve ser erro de compilação ──

#[test]
fn fn_reflection_unknown_field() {
    let output = build_and_expect_error(
        "fn_reflection_unknown_field",
        "soma :: Int Int => Int\nlambda a b: + a b\naction main => Unit\n    echo!(soma.foo)\nmain!()",
    );
    assert!(
        !output.is_empty(),
        "build deve falhar com erro sobre field desconhecido"
    );
}

// ── DoD 15: 42.name deve ser erro (Int não é Function/Action) ──────

#[test]
fn fn_reflection_on_int() {
    let output = build_and_expect_error(
        "fn_reflection_on_int",
        "action main => Unit\n    echo!(42.name)\nmain!()",
    );
    assert!(
        !output.is_empty(),
        "build deve falhar — 42.name não é válido"
    );
}
