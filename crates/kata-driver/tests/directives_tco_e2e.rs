//! Testes E2E — Diretivas customizadas: coexistência com TCO (Tail Call Optimization).
//!
//! PRD-tco-orchestrator: @log{exit}/@log{enter} + TCO coexistem via
//! wrapper/inner split. O wrapper executa synthetic (log) 1 vez; o inner
//! faz TCO (return_call chain, stack O(1)). n=100000 prova TCO.

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
    let dir = std::env::temp_dir().join("kata-driver-directives-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata run <path>` e retorna (stdout, stderr, exit_code).
fn run_kata(path: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .args(["run", path])
        .output()
        .expect("executar kata run");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// DoD 17: `@log{exit}` + TCO — split ativado, exit no wrapper, TCO no inner.
/// Stack O(1) com n=100000. Log dispara 1 vez (não 100000).
#[test]
fn e2e_log_exit_tco_large_n() {
    let src = r#"import stdio

@log{msg: "exit {_return}\n", when: "exit", file: __stdout__}
count_down :: Int Int => Int
lambda 0 acc: acc
lambda n acc: count_down (- n 1) acc

action main => Int
    let r := count_down 100000 0
    echo!(r)
    r

main!()"#;
    let path = write_temp_kata("e2e_log_exit_tco", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(
        code, 0,
        "exit 0 (TCO deve evitar stack overflow com n=100000) — stderr: {stderr}"
    );
    // "exit 0" deve aparecer exatamente 1 vez no stdout (wrapper executa synthetic 1 vez).
    let exit_count = stdout.lines().filter(|l| l.starts_with("exit ")).count();
    assert_eq!(
        exit_count, 1,
        "log{{exit}} deve disparar 1 vez (não {{exit_count}}) — stdout: {stdout}"
    );
    // Resultado correto: count_down 100000 0 = 0.
    assert!(
        stdout.contains("0"),
        "resultado deve ser 0 — stdout: {stdout} | stderr: {stderr}"
    );
}

/// DoD 18: `@log{enter}` + TCO — enter dispara 1 vez no wrapper.
#[test]
fn e2e_log_enter_tco_large_n() {
    let src = r#"import stdio

@log{msg: "enter {_args}\n", when: "enter", file: __stdout__}
count_down :: Int Int => Int
lambda 0 acc: acc
lambda n acc: count_down (- n 1) acc

action main => Int
    let r := count_down 100000 0
    echo!(r)
    r

main!()"#;
    let path = write_temp_kata("e2e_log_enter_tco", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(
        code, 0,
        "exit 0 (TCO deve evitar stack overflow com n=100000) — stderr: {stderr}"
    );
    // "enter (100000, 0)" deve aparecer exatamente 1 vez.
    let enter_count = stdout.lines().filter(|l| l.starts_with("enter ")).count();
    assert_eq!(
        enter_count, 1,
        "log{{enter}} deve disparar 1 vez no wrapper (não {{enter_count}}) — stdout: {stdout}"
    );
}

/// DoD 19: `@log{exit}` + `@cache` + TCO — ambos no wrapper, TCO no inner.
#[test]
fn e2e_log_exit_cache_tco() {
    let src = r#"import stdio

@log{msg: "exit {_return}\n", when: "exit", file: __stdout__}
@cache{strategy: "LRU"}
count_down :: Int Int => Int
lambda 0 acc: acc
lambda n acc: count_down (- n 1) acc

action main => Int
    let r := count_down 100000 0
    echo!(r)
    r

main!()"#;
    let path = write_temp_kata("e2e_log_exit_cache_tco", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(
        code, 0,
        "exit 0 (TCO + cache + log devem coexistir) — stderr: {stderr}"
    );
    // Log dispara 1 vez (no wrapper).
    let exit_count = stdout.lines().filter(|l| l.starts_with("exit ")).count();
    assert_eq!(
        exit_count, 1,
        "log{{exit}} deve disparar 1 vez com @cache + TCO — stdout: {stdout}"
    );
    // Resultado correto.
    assert!(
        stdout.contains("0"),
        "resultado deve ser 0 — stdout: {stdout} | stderr: {stderr}"
    );
}

/// DoD 20: `@cache` + TCO sem @log — regressão do wrapper/inner split
/// (já coberto por cache_tco_large_n em cache_e2e.rs, mas validamos aqui
/// também para garantir que a generalização de needs_split não quebrou).
#[test]
fn e2e_cache_tco_regression_fase5() {
    let src = r#"@cache{strategy: "LRU"}
count_down :: Int Int => Int
lambda 0 acc: acc
lambda n acc: count_down (- n 1) acc

count_down 1000000 1"#;
    let path = write_temp_kata("e2e_cache_tco_fase5", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 (TCO + cache, n=1M) — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "1",
        "count_down 1000000 1 deve ser 1 — stdout: {stdout}"
    );
}
