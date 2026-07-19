//! Testes E2E do subcomando `kata test` — driver descobre, compila e executa
//! wrappers de teste `@test` individualmente.
//!
//! Cada teste cria um arquivo `.kata` temporário, invoca o binário `kata test`
//! via subprocess, e verifica stdout + exit code.
//!
//! Pitfalls (do handoff):
//! - `soma! 1 2` é inválido — parser exige `soma!(1, 2)` (pitfall #45).
//! - `Unit` como Ident não é `Expr::Unit` — usar `()` (pitfall #45).
//! - Panic detection sem catch_unwind: panic = SIGABRT, não capturável
//!   (pitfall #38/#42). Teste de `expects: "Panic: msg"` adiado.
//! - CompileError sub-módulos (C1) não implementados — `expects: "CompileError:"`
//!   reporta `[PENDENTE]` e pula. Teste adiado.

use std::fs;
use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    // CARGO_BIN_EXE_kata é setado pelo cargo em integration tests.
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

/// Executa `kata test <path>` e retorna (stdout, exit_code).
fn run_kata_test(path: &str) -> (String, i32) {
    let output = Command::new(kata_bin())
        .args(["test", path])
        .output()
        .expect("executar kata test");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (stdout, output.status.code().unwrap_or(-1))
}

// ── Teste 1: @test sem args, sem expects — sucesso ──

/// Action sem params com `@test("desc")` — wrapper executa, retorna
/// resultado, driver reporta `[PASS]`.
#[test]
fn test_sem_args_passa() {
    let path = write_temp_kata(
        "test_sem_args_passa",
        r#"@test("resposta")
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
        "deve citar a action — stdout: {stdout}"
    );
    assert!(
        stdout.contains("1 passed"),
        "deve ter 1 passed — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 quando todos passam — stdout: {stdout}");
}

// ── Teste 2: @test com args compostos (tupla) — sucesso ──

/// Action com 2 params Int, `@test{args: (3, 4)}` — wrapper lowera
/// args como tupla na arena, spawn passa args_ptr, action lê params.
#[test]
fn test_com_args_tupla_passa() {
    let path = write_temp_kata(
        "test_com_args_tupla_passa",
        r#"@test{desc: "soma 3+4", args: (3, 4)}
action soma (a::Int, b::Int) => Int
    + a b
soma!(1, 2)"#,
    );

    let (stdout, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "deve ter [PASS] — stdout: {stdout}"
    );
    assert!(
        stdout.contains("1 passed"),
        "deve ter 1 passed — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── Teste 3: @test com timeout — falha por timeout ──

/// Action com loop infinito e `@test{timeout: 100}` — thread OS spawna,
/// `TIMEOUT_EXPIRED` seta após 100ms, `yield_check` slow path dispara
/// `YieldReason::Timeout`, wrapper retorna `TIMEOUT_SENTINEL`.
#[test]
fn test_timeout_falha() {
    let path = write_temp_kata(
        "test_timeout_falha",
        r#"@test{desc: "loop infinito", timeout: 100}
action infinito => Unit
    var i := 0
    loop
        i := + i 1
infinito!()"#,
    );

    let (stdout, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[TIMEOUT]"),
        "deve ter [TIMEOUT] — stdout: {stdout}"
    );
    assert!(
        stdout.contains("1 failed"),
        "deve ter 1 failed — stdout: {stdout}"
    );
    assert_eq!(code, 1, "exit 1 quando há falhas — stdout: {stdout}");
}

// ── Teste 4: múltiplos @test na mesma action ──

/// Dois `@test` na mesma action geram dois wrappers. Ambros passam,
/// driver reporta 2 passed.
#[test]
fn test_multiplos_casos_mesma_action() {
    let path = write_temp_kata(
        "test_multiplos_casos_mesma_action",
        r#"@test{desc: "caso 3+4", args: (3, 4)}
@test{desc: "caso 10+20", args: (10, 20)}
action soma (a::Int, b::Int) => Int
    + a b
soma!(1, 2)"#,
    );

    let (stdout, code) = run_kata_test(&path);

    assert!(
        stdout.contains("[PASS]"),
        "deve ter [PASS] — stdout: {stdout}"
    );
    assert!(
        stdout.contains("caso 3+4"),
        "deve citar caso 3+4 — stdout: {stdout}"
    );
    assert!(
        stdout.contains("caso 10+20"),
        "deve citar caso 10+20 — stdout: {stdout}"
    );
    assert!(
        stdout.contains("2 passed"),
        "deve ter 2 passed — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 quando todos passam — stdout: {stdout}");
}

// ── Teste 5: --filter por substring ──

/// `--filter` filtra testes por substring na descrição. Testes que
/// não casam o filtro são skipados (contam como skipped).
#[test]
fn test_filter_por_substring() {
    let path = write_temp_kata(
        "test_filter_por_substring",
        r#"@test{desc: "soma rapida", args: (3, 4)}
@test{desc: "soma lenta", args: (10, 20)}
action soma (a::Int, b::Int) => Int
    + a b
soma!(1, 2)"#,
    );

    // Filtra por "rapida" — só o primeiro teste deve rodar.
    let output = Command::new(kata_bin())
        .args(["test", &path, "--filter", "rapida"])
        .output()
        .expect("executar kata test --filter");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let code = output.status.code().unwrap_or(-1);

    assert!(
        stdout.contains("soma rapida"),
        "deve citar soma rapida — stdout: {stdout}"
    );
    assert!(
        !stdout.contains("[PASS].*soma lenta"),
        "não deve executar soma lenta — stdout: {stdout}"
    );
    assert!(
        stdout.contains("1 passed"),
        "deve ter 1 passed — stdout: {stdout}"
    );
    assert!(
        stdout.contains("1 skipped"),
        "deve ter 1 skipped — stdout: {stdout}"
    );
    assert_eq!(code, 0, "exit 0 — stdout: {stdout}");
}

// ── Testes adiados (sem mecanismo ainda) ──────────────────────────

/// `expects: "Panic: msg"` exige detecção de panic sem abortar o processo.
/// O runtime usa `extern "C"` (nounwind em Rust 2024) — panic vira SIGABRT,
/// não capturável via `#[should_panic]` ou valor de retorno. Necessário
/// `catch_unwind` no runtime ou panic sentinel — não implementado.
/// Ver handoff pitfall #38/#42.
#[test]
#[ignore = "panic detection sem catch_unwind — mecanismo não implementado"]
fn test_expects_panic_adiado() {}

/// `@test` sem `args` em action que recebe params deve falhar com erro
/// claro de codegen, não SIGSEGV em runtime. O wrapper passa `args_ptr = 0`
/// (null) quando `@test` não tem `args` — a action lê params de null →
/// null dereference. Validação no codegen detecta o mismatch antes do JIT.
#[test]
fn test_sem_args_em_action_com_params_falha_graciosamente() {
    let path = write_temp_kata(
        "test_sem_args_em_action_com_params_falha_graciosamente",
        r#"@test("soma sem args")
action soma (a::Int, b::Int) => Int
    + a b
soma!(1, 2)"#,
    );

    let output = Command::new(kata_bin())
        .args(["test", &path])
        .output()
        .expect("executar kata test");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Driver deve falhar com erro de codegen (não SIGSEGV/signal).
    assert!(
        stderr.contains("args") && stderr.contains("soma"),
        "deve reportar erro sobre args faltando em soma — stderr: {stderr}"
    );
    // SIGSEGV produz exit code 139 (128 + 11). Erro normal produz exit 1.
    let code = output.status.code().unwrap_or(-1);
    assert_ne!(code, 139, "não deve ser SIGSEGV (exit 139) — code: {code}");
    assert_eq!(code, 1, "deve ser exit 1 (erro de codegen) — code: {code}");
}

/// `expects: "CompileError: msg"` exige que o driver compile um sub-módulo
/// isolado e verifique que a inferência/codegen falha com o erro esperado.
/// O design C1 (sub-módulos isolados) não tem fase atribuída — o driver
/// atual reporta `[PENDENTE]` e pula. Ver handoff ponto de revisão 3.
#[test]
#[ignore = "sub-módulos isolados (C1) não implementados"]
fn test_expects_compileerror_adiado() {}
