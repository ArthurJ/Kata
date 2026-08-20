//! Testes E2E de doctests — `kata test` com `>>> ` em comentários `#{ }#`.
//!
//! Cada teste cria um arquivo `.kata` temporário, invoca `kata test` via
//! subprocess, e verifica stdout + exit code.

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
    let dir = std::env::temp_dir().join("kata-driver-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata test <path>` e retorna (stdout, stderr, exit_code).
fn run_kata_test(path: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .args(["test", path])
        .output()
        .expect("executar kata test");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);
    (stdout, stderr, code)
}

// ── 1: Doctest simples — PASS ──

#[test]
fn doctest_simples_passa() {
    let path = write_temp_kata(
        "doctest_simples_passa",
        r#"#{
>>> constant x := 42
>>> x
42
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(stdout.contains("[PASS]"), "deve ter [PASS] — stdout: {stdout}");
    assert!(
        stdout.contains("doctest linha 2"),
        "deve citar linha 2 — stdout: {stdout}"
    );
    assert!(
        stdout.contains("doctest linha 3"),
        "deve citar linha 3 — stdout: {stdout}"
    );
    assert!(
        stdout.contains("2 passed"),
        "deve ter 2 passed — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── 2: Doctest com output mismatch — FAIL ──

#[test]
fn doctest_output_mismatch_falha() {
    let path = write_temp_kata(
        "doctest_output_mismatch_falha",
        r#"#{
>>> constant x := 10
>>> x
99
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[FAIL]"),
        "deve ter [FAIL] — stdout: {stdout}"
    );
    assert!(
        stdout.contains("output mismatch"),
        "deve citar output mismatch — stdout: {stdout}"
    );
    assert!(
        stdout.contains("esperado: 99"),
        "deve mostrar esperado — stdout: {stdout}"
    );
    assert!(
        stdout.contains("obtido:   10"),
        "deve mostrar obtido — stdout: {stdout}"
    );
    assert_eq!(code, 1, "exit 1 quando falha — stdout: {stdout}");
}

// ── 3: Múltiplos blocos separados por linha vazia ──

#[test]
fn doctest_multiplos_blocos_separados() {
    let path = write_temp_kata(
        "doctest_multiplos_blocos",
        r#"#{
>>> constant x := 10
>>> x
10

>>> constant y := 20
>>> y
20
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(stdout.contains("4 passed"), "deve ter 4 passed — stdout: {stdout}");
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── 4: Segundo bloco não vê bindings do primeiro ──

#[test]
fn doctest_segundo_bloco_nao_ve_bindings() {
    let path = write_temp_kata(
        "doctest_segundo_bloco_isolado",
        r#"#{
>>> constant x := 10
>>> x
10

>>> x
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    // O segundo bloco deve falhar — x não está no escopo
    assert!(
        stdout.contains("1 failed"),
        "deve ter 1 failed (x nao visivel no 2o bloco) — stdout: {stdout}"
    );
    assert!(
        stdout.contains("nao esta no escopo") || stdout.contains("não está no escopo"),
        "deve citar erro de escopo — stdout: {stdout}"
    );
    assert_eq!(code, 1, "exit 1 — stdout: {stdout}");
}

// ── 5: echo! capturado ──

#[test]
fn doctest_echo_capturado() {
    let path = write_temp_kata(
        "doctest_echo",
        r#"#{
>>> echo!("hello")
hello
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(stdout.contains("[PASS]"), "deve passar — stdout: {stdout}");
    assert!(stdout.contains("1 passed"), "deve ter 1 passed — stdout: {stdout}");
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── 6: @test e doctest no mesmo arquivo ──

#[test]
fn doctest_e_at_test_mesmo_arquivo() {
    let path = write_temp_kata(
        "doctest_e_at_test",
        r#"#{
>>> constant x := 42
>>> x
42
}#

@test("soma")
action soma => Int
    + 1 2
soma!()"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    // Doctest + @test = 3 passed
    assert!(stdout.contains("3 passed"), "deve ter 3 passed — stdout: {stdout}");
    assert!(
        stdout.contains("doctest linha 2"),
        "deve citar doctest linha 2 — stdout: {stdout}"
    );
    assert!(
        stdout.contains("soma"),
        "deve citar @test soma — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── 7: Supressão de Unit — let sem output esperado ──

#[test]
fn doctest_let_sem_output() {
    let path = write_temp_kata(
        "doctest_let_sem_output",
        r#"#{
>>> let x := 42
>>> x
42
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    // `let x := 42` não produz output (Unit suprimido)
    // `x` produz 42
    assert!(stdout.contains("2 passed"), "deve ter 2 passed — stdout: {stdout}");
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── 8: Comentário sem `>>> ` é ignorado ──

#[test]
fn doctest_comentario_sem_marcador_ignorado() {
    let path = write_temp_kata(
        "doctest_comentario_sem_marcador",
        r#"#{
Isto é apenas documentação.
Sem doctests aqui.
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    // Nenhum doctest, nenhum @test — 0 passed
    assert!(stdout.contains("0 passed"), "deve ter 0 passed — stdout: {stdout}");
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── 9: Arquivo só com doctests (sem código executável) ──

#[test]
fn doctest_sem_codigo_executavel() {
    let path = write_temp_kata(
        "doctest_sem_codigo",
        r#"#{
>>> constant x := 42
>>> x
42
}#"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    // Doctests passam mesmo sem código executável
    assert!(stdout.contains("2 passed"), "deve ter 2 passed — stdout: {stdout}");
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── 10: Input multiline com match ──

#[test]
fn doctest_multiline_match() {
    let path = write_temp_kata(
        "doctest_multiline_match",
        r#"#{
>>> match True
  True: "sim"
  False: "nao"
sim
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(stdout.contains("[PASS]"), "deve passar — stdout: {stdout}");
    assert!(stdout.contains("1 passed"), "deve ter 1 passed — stdout: {stdout}");
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── 11: Texto livre antes de doctests ──

#[test]
fn doctest_texto_livre_antes() {
    let path = write_temp_kata(
        "doctest_texto_livre_antes",
        r#"#{
Calcula fatorial recursivamente.
Veja: https://exemplo.com/fatorial

>>> constant x := 42
>>> x
42
}#
echo!(0)"#,
    );

    let (stdout, _stderr, code) = run_kata_test(&path);

    assert!(stdout.contains("2 passed"), "deve ter 2 passed — stdout: {stdout}");
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}