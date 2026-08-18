//! Testes E2E do `kata build` — compilação AOT + linker.
//!
//! Cada teste:
//! 1. Escreve um arquivo `.kata` temporário
//! 2. Executa `kata build` para produzir um executável
//! 3. Executa o executável e verifica o stdout
//!
//! `libkata_rt.a` e `libkata_rt.so` são compiladas automaticamente no
//! setup antes do primeiro teste. `cc` deve estar disponível no PATH.

use std::fs;
use std::process::Command;
use std::sync::Once;

static SETUP: Once = Once::new();

/// Setup: garante que `libkata_rt.a` (staticlib) e `libkata_rt.so` (cdylib)
/// existam em `target/<profile>/`. `cargo test` compila `kata-rt` apenas
/// como `rlib` para link interno — o `staticlib`/`cdylib` precisa ser
/// produzido explicitamente. Os testes rodam após o cargo liberar o lock
/// do workspace, então `cargo build -p kata-rt` não deadlocka.
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

/// Retorna o path do binário `kata` compilado pelo cargo.
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Cria um arquivo `.kata` temporário e retorna o path.
fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e-build");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata build <input> -o <output>` e retorna (stdout, exit_code).
fn run_kata_build(input: &str, output: &str) -> (String, i32) {
    ensure_kata_rt_built();
    let output_path = std::env::temp_dir()
        .join("kata-driver-e2e-build")
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

/// Executa o binário produzido por `kata build` e retorna (stdout, exit_code).
fn run_built_binary(name: &str) -> (String, i32) {
    let bin_path = std::env::temp_dir()
        .join("kata-driver-e2e-build")
        .join(name);
    let result = Command::new(&bin_path)
        .output()
        .expect("executar binário AOT");
    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    (stdout, result.status.code().unwrap_or(-1))
}

// ── Teste 1: Int (fatorial recursivo com TCO) ────────────────

/// `fat 5 1` → 120. Testa o caminho básico: SMI untag, Int display.
#[test]
fn build_int_fatorial() {
    let src =
        "fat :: Int Int => Int\nlambda 0 acc: acc\nlambda n acc: fat (- n 1) (* n acc)\nfat 5 1";
    let path = write_temp_kata("build_int_fatorial", src);
    let (build_out, build_code) = run_kata_build(&path, "build_int_fatorial_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_int_fatorial_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    assert_eq!(stdout.trim(), "120", "fatorial 5 deve imprimir 120");
}

// ── Teste 2: Float ──────────────────────────────────────────

/// `+ 3.14 1.0` → 4.140000000000001. Testa o caminho Float: XMM0 retorno,
/// bitcast para i64, from_bits no display.
#[test]
fn build_float() {
    let path = write_temp_kata("build_float", "+ 3.14 1.0");
    let (build_out, build_code) = run_kata_build(&path, "build_float_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_float_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    assert_eq!(
        stdout.trim(),
        "4.140000000000001",
        "float deve imprimir 4.140000000000001"
    );
}

// ── Teste 3: Text literal ────────────────────────────────────

/// `"hello world"` → hello world. Testa declare_data + define_data
/// para strings literais no AOT.
#[test]
fn build_text_literal() {
    let path = write_temp_kata("build_text_literal", "\"hello world\"");
    let (build_out, build_code) = run_kata_build(&path, "build_text_literal_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_text_literal_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    assert_eq!(
        stdout.trim(),
        "hello world",
        "text literal deve imprimir hello world"
    );
}

// ── Teste 4: Boolean (match) ─────────────────────────────────

/// `match True` com braços True: 1, False: 0 → 1.
#[test]
fn build_boolean() {
    let src = "match True\n   True: 1\n   False: 0";
    let path = write_temp_kata("build_boolean", src);
    let (build_out, build_code) = run_kata_build(&path, "build_boolean_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_boolean_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    assert_eq!(stdout.trim(), "1", "match True deve imprimir 1");
}

// ── Teste 5: Action com echo! (Unit) ─────────────────────────

/// Action com echo! imprime "hello" e "world". O resultado é Unit.
#[test]
fn build_action_echo() {
    let src = "action greet\n    echo!(\"hello\")\n    echo!(\"world\")\ngreet!()";
    let path = write_temp_kata("build_action_echo", src);
    let (build_out, build_code) = run_kata_build(&path, "build_action_echo_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_action_echo_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    // echo! imprime com newline, Unit imprime "()" — resultado: "hello\nworld\n()"
    assert_eq!(
        stdout.trim(),
        "hello\nworld\n()",
        "action echo deve imprimir hello\\nworld\\n()"
    );
}

// ── Teste 6: BigInt (overflow SMI) ───────────────────────────

/// `* 100000000000000000000 2` → 200000000000000000000.
/// Testa o caminho BigInt: valor não cabe em SMI, usa ponteiro BigInt,
/// kata_rt_bi_show no display.
#[test]
fn build_bigint() {
    let path = write_temp_kata("build_bigint", "* 100000000000000000000 2");
    let (build_out, build_code) = run_kata_build(&path, "build_bigint_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_bigint_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    assert_eq!(
        stdout.trim(),
        "200000000000000000000",
        "bigint deve imprimir 200000000000000000000"
    );
}

// ── Teste 7: --dynamic (link dinâmico) ──────────────────────

/// `kata build --dynamic` produz executável que linka com libkata_rt.so.
/// O resultado deve ser o mesmo do link estático.
#[test]
fn build_dynamic() {
    ensure_kata_rt_built();
    let src =
        "fat :: Int Int => Int\nlambda 0 acc: acc\nlambda n acc: fat (- n 1) (* n acc)\nfat 5 1";
    let path = write_temp_kata("build_dynamic", src);
    let output_path = std::env::temp_dir()
        .join("kata-driver-e2e-build")
        .join("build_dynamic_bin");
    let output_str = output_path.to_string_lossy().to_string();

    let result = Command::new(kata_bin())
        .args(["build", &path, "-o", &output_str, "--dynamic"])
        .output()
        .expect("executar kata build --dynamic");
    let stderr = String::from_utf8_lossy(&result.stderr).to_string();
    assert_eq!(
        result.status.code(),
        Some(0),
        "kata build --dynamic deve exit 0 — stderr: {stderr}"
    );

    let bin_result = Command::new(&output_path)
        .output()
        .expect("executar binário dinâmico");
    let stdout = String::from_utf8_lossy(&bin_result.stdout).to_string();
    assert_eq!(stdout.trim(), "120", "dynamic build deve imprimir 120");
}

// ── Teste 8: Rational ────────────────────────────────────────

/// `3.14::Rational` → 3.14. Testa o caminho Rational: ponteiro para
/// BigRational, kata_rt_rat_show no display.
#[test]
fn build_rational() {
    let path = write_temp_kata("build_rational", "3.14::Rational");
    let (build_out, build_code) = run_kata_build(&path, "build_rational_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_rational_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    assert_eq!(stdout.trim(), "3.14", "rational deve imprimir 3.14");
}

// ── Teste 9: Range Int com step default ─────────────────────

/// `[0..3]` com for-in → 0, 1, 2. Testa step default (STEPPABLE)
/// para Int: step=1, exclusive.
#[test]
fn build_range_int_step_default() {
    let src = "action main\n    for x in [0..3]\n        echo!(x)\nmain!()";
    let path = write_temp_kata("build_range_int_step_default", src);
    let (build_out, build_code) = run_kata_build(&path, "build_range_int_step_default_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_range_int_step_default_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    let lines: Vec<&str> = stdout.trim().lines().filter(|l| *l != "()").collect();
    assert_eq!(lines, vec!["0", "1", "2"], "range [0..3] deve iterar 0,1,2");
}

// ── Teste 10: Range Int com step default inclusive ───────────

/// `[0..=3]` com for-in → 0, 1, 2, 3. Testa step default + inclusive.
#[test]
fn build_range_int_step_default_inclusive() {
    let src = "action main\n    for x in [0..=3]\n        echo!(x)\nmain!()";
    let path = write_temp_kata("build_range_int_step_default_inclusive", src);
    let (build_out, build_code) =
        run_kata_build(&path, "build_range_int_step_default_inclusive_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_range_int_step_default_inclusive_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    let lines: Vec<&str> = stdout.trim().lines().filter(|l| *l != "()").collect();
    assert_eq!(
        lines,
        vec!["0", "1", "2", "3"],
        "range [0..=3] deve iterar 0,1,2,3"
    );
}

// ── Teste 11: Range Float com step explícito ─────────────────

/// `[0.0..0.5..2.0]` com for-in → 0.0, 0.5, 1.0, 1.5. Testa
/// codegen de Float no range: fcmp para condição de parada,
/// fadd para avanço.
#[test]
fn build_range_float_explicit_step() {
    let src = "action main\n    for x in [0.0..0.5..2.0]\n        echo!(x)\nmain!()";
    let path = write_temp_kata("build_range_float_explicit_step", src);
    let (build_out, build_code) = run_kata_build(&path, "build_range_float_explicit_step_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_range_float_explicit_step_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    let lines: Vec<&str> = stdout.trim().lines().filter(|l| *l != "()").collect();
    assert_eq!(
        lines,
        vec!["0.0", "0.5", "1.0", "1.5"],
        "range [0.0..0.5..2.0] deve iterar 0.0,0.5,1.0,1.5"
    );
}

// ── Teste 12: Range Float com step default ───────────────────

/// `[0.0..3.0]` com for-in → 0.0, 1.0, 2.0. Testa step default
/// via STEPPABLE para Float: step=1.0, exclusive.
#[test]
fn build_range_float_step_default() {
    let src = "action main\n    for x in [0.0..3.0]\n        echo!(x)\nmain!()";
    let path = write_temp_kata("build_range_float_step_default", src);
    let (build_out, build_code) = run_kata_build(&path, "build_range_float_step_default_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("build_range_float_step_default_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    let lines: Vec<&str> = stdout.trim().lines().filter(|l| *l != "()").collect();
    assert_eq!(
        lines,
        vec!["0.0", "1.0", "2.0"],
        "range [0.0..3.0] deve iterar 0.0,1.0,2.0"
    );
}
