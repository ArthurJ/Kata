//! Testes E2E do `#!test` (Fase 3 do PRD-pragma-mechanism).
//!
//! T11: `#!test("desc")` antes de action → `kata test` descobre e executa.
//! T12: `@test{expects}` permanece funcionando como diretiva (não migra).
//! T13: `#!test` sem expects não gera wrapper de verificação (passa ao completar).
//! T14: `#!test{desc, args, timeout}` — TestSpec com args e timeout populados.
//!
//! Cada teste cria um arquivo `.kata` temporário, invoca o binário `kata test`
//! via subprocess, e verifica stdout + exit code.

use std::fs;
use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Cria um arquivo `.kata` temporário e retorna o path.
fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-pragma-test-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata test <path>` e retorna (stdout, exit_code).
fn run_kata_test(path: &str) -> (String, i32) {
    let output = Command::new(kata_bin())
        .args(["test", path])
        .output()
        .expect("executar kata test");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (stdout, output.status.code().unwrap_or(-1))
}

// ── T11: #!test marker — descoberto e executado ──

/// `#!test("soma")` antes de action → `kata test` descobre e executa.
/// Mesmo comportamento que `@test("soma")` atual.
#[test]
fn t11_pragma_test_marker_descoberto_e_executado() {
    let path = write_temp_kata(
        "t11_pragma_test_marker",
        r#"#!test("resposta")
action resposta => Int
    42
resposta!()"#,
    );

    let (stdout, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "deve ter [PASS] — stdout: {stdout}"
    );
    assert!(
        stdout.contains("resposta"),
        "deve citar a descrição do teste — stdout: {stdout}"
    );
    assert!(
        stdout.contains("1 passed"),
        "deve ter 1 passed — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 quando todos passam — stdout: {stdout}");
}

// ── T12: @test{expects} permanece diretiva ──

/// `@test{desc, expects}` continua funcionando como diretiva `@`
/// (gera wrapper com verificação ativa de show(err) vs expects).
/// Não migra para `#!` porque gera código — é extensão de funcionalidade.
#[test]
fn t12_test_expects_permence_diretiva() {
    let path = write_temp_kata(
        "t12_test_expects_permence",
        r#"enum MeuErro
    ValidacaoFail

@test{desc: "valida", expects: "ValidacaoFail", policy: prefix, args: ("chk")}
action valida (url::Text) => Result::(Text, MeuErro)
    Result::Err MeuErro::ValidacaoFail

valida!("chk")"#,
    );

    let (stdout, code) = run_kata_test(&path);

    // expects deve casar com "ValidacaoFail" (prefix) — [PASS]
    assert!(
        stdout.contains("[PASS]"),
        "deve ter [PASS] (expects casa) — stdout: {stdout}"
    );
    assert!(
        stdout.contains("valida"),
        "deve citar a descrição — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── T13: #!test sem expects — passa ao completar ──

/// `#!test("soma")` sem expects → wrapper retorna resultado bruto,
/// não extrai tag nem compara string. Passa se completa.
#[test]
fn t13_pragma_test_sem_expects_passa_ao_completar() {
    let path = write_temp_kata(
        "t13_pragma_test_sem_expects",
        r#"#!test("soma simples")
action soma => Int
    + 2 3

soma!()"#,
    );

    let (stdout, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "deve ter [PASS] — stdout: {stdout}"
    );
    assert!(
        stdout.contains("soma simples"),
        "deve citar a descrição — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── T14: #!test com args e timeout — TestSpec populado ──

/// `#!test{desc: "com args", args: (2, 3), timeout: 5000}` → TestSpec
/// com args e timeout populados, runner passa args para a action.
#[test]
fn t14_pragma_test_com_args_e_timeout() {
    let path = write_temp_kata(
        "t14_pragma_test_args_timeout",
        r#"#!test{desc: "soma com args", args: (2, 3), timeout: 5000}
action soma (a::Int, b::Int) => Int
    + a b

soma!(2, 3)"#,
    );

    let (stdout, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "deve ter [PASS] — stdout: {stdout}"
    );
    assert!(
        stdout.contains("soma com args"),
        "deve citar a descrição com args — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── Extra: #!test e @test no mesmo arquivo ──

/// Um arquivo com ambos `#!test` (marker) e `@test{expects}` (assertion)
/// — ambos devem ser descobertos e executados.
#[test]
fn pragma_test_e_diretiva_test_coexistem() {
    let path = write_temp_kata(
        "pragma_test_e_diretiva_coexistem",
        r#"#!test("marker puro")
action simples => Int
    42

enum MeuErro
    ValidacaoFail

@test{desc: "com expects", expects: "ValidacaoFail", policy: prefix, args: ("chk")}
action valida (url::Text) => Result::(Text, MeuErro)
    Result::Err MeuErro::ValidacaoFail

simples!()
valida!("chk")"#,
    );

    let (stdout, _code) = run_kata_test(&path);

    // Ambos devem aparecer no output
    assert!(
        stdout.contains("marker puro"),
        "deve citar #!test — stdout: {stdout}"
    );
    assert!(
        stdout.contains("com expects"),
        "deve citar @test{{expects}} — stdout: {stdout}"
    );
    assert!(
        stdout.contains("2 passed"),
        "deve ter 2 passed — stdout: {stdout}"
    );
}
