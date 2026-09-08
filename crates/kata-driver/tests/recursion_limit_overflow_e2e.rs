//! Testes E2E: Overflow de recursão aborta o processo via kata_rt_overflow_panic.
//!
//! O overflow_block do codegen agora chama kata_rt_overflow_panic (que faz
//! process::exit(1) com mensagem estruturada) em vez de retornar um dummy 0.
//! Estes testes validam via subprocess que:
//! 1. O processo aborta com exit code != 0
//! 2. A mensagem "recursion depth exceeded" aparece no stderr
//! 3. Nenhum dummy 0 é impresso no stdout
//!
//! Estes testes vivem em kata-driver (não kata-codegen) porque precisam do
//! binário `kata` via CARGO_BIN_EXE_kata. O overflow faz process::exit,
//! matando o processo — não pode ser testado in-process.

use std::process::Command;

/// Localiza o binário `kata` compilado.
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata run` com o conteúdo dado e retorna (stdout, stderr, exit_code).
fn run_kata(source: &str) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_overflow_e2e_{id}_{pid}.kata",
        pid = std::process::id()
    ));
    std::fs::write(&path, source).expect("escrever arquivo temporário");
    let output = Command::new(kata_bin())
        .args(["run", &path.to_string_lossy()])
        .output()
        .expect("executar kata run");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// Recursão não-de-cauda com profundidade > limite aborta o processo.
/// `count 1200` com limite default 1000 → exit != 0, stderr com mensagem,
/// stdout sem dummy 0.
#[test]
fn overflow_direct_recursion_aborts() {
    let src = "\
count :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (count (- n 1)) 1

count 1200";
    let (stdout, stderr, code) = run_kata(src);
    assert_ne!(
        code, 0,
        "overflow deve abortar com exit != 0 — code: {code}"
    );
    assert!(
        stderr.contains("recursion depth exceeded"),
        "stderr deve mencionar recursion depth — stderr: {stderr}"
    );
    assert!(
        !stdout.contains('0'),
        "não deve imprimir dummy 0 no stdout — stdout: {stdout}"
    );
}

/// Recursão mútua não-de-cauda com profundidade > limite aborta o processo.
#[test]
fn overflow_mutual_recursion_aborts() {
    let src = "\
ping :: Int => Int
lambda 0: 0
lambda n: + (pong (- n 1)) 1

pong :: Int => Int
lambda 0: 0
lambda n: + (ping (- n 1)) 1

ping 2000";
    let (stdout, stderr, code) = run_kata(src);
    assert_ne!(code, 0, "overflow mútuo deve abortar — code: {code}");
    assert!(
        stderr.contains("recursion depth exceeded"),
        "stderr deve mencionar recursion depth — stderr: {stderr}"
    );
    assert!(
        !stdout.contains('0'),
        "não deve imprimir dummy 0 — stdout: {stdout}"
    );
}

/// call_indirect (não-de-cauda) também é contado e aborta no overflow.
#[test]
fn overflow_indirect_recursion_aborts() {
    let src = "\
count :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (count (- n 1)) 1

apply :: (Int -> Int) Int => Int
lambda f x: f x

action main
    echo!(apply (count) 1200)
main!()";
    let (stdout, stderr, code) = run_kata(src);
    assert_ne!(code, 0, "overflow indireto deve abortar — code: {code}");
    assert!(
        stderr.contains("recursion depth exceeded"),
        "stderr deve mencionar recursion depth — stderr: {stderr}"
    );
    assert!(
        !stdout.contains('0'),
        "não deve imprimir dummy 0 — stdout: {stdout}"
    );
}

/// `fib 1001` com @cache (recursão ramificada) sem config — aborta sem
/// imprimir dummy 0. Este é o caso canônico do bug: antes do fix, o echo!
/// imprimia 0 antes do erro ser detectado.
#[test]
fn overflow_fib_ramified_no_dummy_output() {
    let src = "\
@cache
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

echo!(fib 1001)";
    let (stdout, stderr, code) = run_kata(src);
    assert_ne!(code, 0, "fib 1001 deve abortar — code: {code}");
    assert!(
        stderr.contains("recursion depth exceeded"),
        "stderr deve mencionar recursion depth — stderr: {stderr}"
    );
    // O bug original: echo! imprimia 0 antes do erro.
    // Agora o overflow_panic aborta antes do echo! produzir output.
    assert!(
        stdout.trim().is_empty(),
        "stdout deve estar vazio (sem dummy 0) — stdout: {stdout}"
    );
}
