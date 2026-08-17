//! Testes E2E de `expects` com `policy` no test runner.
//!
//! Verifica que o wrapper gerado faz `show` no payload de `Result::Err`
//! e compara contra a string `expects` usando a política de match
//! (exact, prefix, contains).
//!
//! Fluxo: cria arquivo `.kata` temporário, executa `kata test`, verifica
//! stdout + exit code.

use std::fs;
use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

fn run_kata_test(path: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .args(["test", path])
        .output()
        .expect("executar kata test");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.code().unwrap_or(-1))
}

// ── Teste 1: expects com policy prefix — Err com variante unitária → PASS ──

#[test]
fn expects_prefix_variante_unitaria_passa() {
    let path = write_temp_kata(
        "expects_prefix_unit_passa",
        r#"enum MeuErro
    Timeout
    ValidacaoFail

@test{desc: "timeout", expects: "Timeout", policy: prefix, args: ("http://slow.example")}
action buscar (url::Text) => Result::(Text, MeuErro)
    Result::Err MeuErro::Timeout
buscar!("http://slow.example")"#,
    );

    let (stdout, stderr, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "prefix match deve passar — stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout} stderr: {stderr}");
}

// ── Teste 2: expects com policy prefix — Err com variante errada → FAIL ──

#[test]
fn expects_prefix_variante_errada_falha() {
    let path = write_temp_kata(
        "expects_prefix_errada_falha",
        r#"enum MeuErro
    Timeout
    ValidacaoFail

@test{desc: "timeout", expects: "Timeout", policy: prefix, args: ("http://slow.example")}
action buscar (url::Text) => Result::(Text, MeuErro)
    Result::Err MeuErro::ValidacaoFail
buscar!("http://slow.example")"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[FAIL]"),
        "prefix mismatch deve falhar — stdout: {stdout}"
    );
    assert_eq!(code, 1, "exit 1 — stdout: {stdout}");
}

// ── Teste 3: expects sem policy (default exact) — match exato → PASS ──

#[test]
fn expects_default_exact_passa() {
    let path = write_temp_kata(
        "expects_default_exact_passa",
        r#"enum MeuErro
    Timeout

@test{desc: "timeout", expects: "Timeout", args: ("http://slow.example")}
action buscar (url::Text) => Result::(Text, MeuErro)
    Result::Err MeuErro::Timeout
buscar!("http://slow.example")"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "exact match default deve passar — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── Teste 4: expects com policy exact — string não casa → FAIL ──

#[test]
fn expects_exact_nao_casa_falha() {
    let path = write_temp_kata(
        "expects_exact_nao_casa_falha",
        r#"enum MeuErro
    Timeout

@test{desc: "timeout", expects: "TimeoutXYZ", policy: exact, args: ("http://slow.example")}
action buscar (url::Text) => Result::(Text, MeuErro)
    Result::Err MeuErro::Timeout
buscar!("http://slow.example")"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[FAIL]"),
        "exact mismatch deve falhar — stdout: {stdout}"
    );
    assert_eq!(code, 1, "exit 1 — stdout: {stdout}");
}

// ── Teste 5: action retorna Ok com expects → FAIL (expected Err, got Ok) ──

#[test]
fn expects_ok_quando_esperava_err_falha() {
    let path = write_temp_kata(
        "expects_ok_falha",
        r#"enum MeuErro
    Timeout

@test{desc: "timeout", expects: "Timeout", policy: prefix, args: ("http://ok.example")}
action buscar (url::Text) => Result::(Text, MeuErro)
    Result::Ok "sucesso"
buscar!("http://ok.example")"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[FAIL]"),
        "Ok quando esperava Err deve falhar — stdout: {stdout}"
    );
    assert!(
        stdout.contains("expected Err, got Ok"),
        "mensagem deve dizer expected Err got Ok — stdout: {stdout}"
    );
    assert_eq!(code, 1, "exit 1 — stdout: {stdout}");
}

// ── Teste 6: sem expects — comportamento atual (pass se completa) ──

#[test]
fn sem_expects_passa_normal() {
    let path = write_temp_kata(
        "sem_expects_passa",
        r#"@test("resposta")
action resposta => Int
    42
resposta!()"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "sem expects deve passar normalmente — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── Teste 7: expects com policy contains — substring → PASS ──

#[test]
fn expects_contains_passa() {
    let path = write_temp_kata(
        "expects_contains_passa",
        r#"enum MeuErro
    Timeout
    ValidacaoFail

@test{desc: "validacao", expects: "Validacao", policy: contains, args: ("http://bad.example")}
action buscar (url::Text) => Result::(Text, MeuErro)
    Result::Err MeuErro::ValidacaoFail
buscar!("http://bad.example")"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "contains match deve passar — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── Teste 8: expects sem Result → compile error (não warning, não SIGSEGV) ──

#[test]
fn expects_sem_result_e_compile_error() {
    let path = write_temp_kata(
        "expects_sem_result_erro",
        r#"@test{desc: "non-result", expects: "something", args: ()}
action resposta => Int
    42
resposta!()"#,
    );

    let (stdout, stderr, code) = run_kata_test(&path);

    // Deve falhar com compile error (não SIGSEGV, não passar).
    assert!(
        stderr.contains("type.mismatch") || stderr.contains("expects"),
        "esperava compile error sobre expects sem Result — stderr: {stderr}"
    );
    assert_ne!(code, 0, "não deve passar — exit code: {code}");
    assert_ne!(code, 139, "não deve SIGSEGV — exit code: {code}");
}