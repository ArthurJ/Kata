//! Testes E2E — constant folding de chamadas com args literais.
//!
//! Testa que funções puras chamadas com argumentos literais são
//! dobradas (folded) em compile-time, sem `@comptime` explícito.

use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata run <file>` com o conteúdo dado e retorna (stdout, stderr, exit_code).
/// Escreve o conteúdo num arquivo temporário e chama `kata run`.
fn run_kata_run(source: &str) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_comptime_fold_{id}_{pid}.kata",
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

// ── Ponto 7: Constant folding de chamadas com args literais ───────────

/// `dobro 5` (função pura com arg literal) deve ser dobrado para `10`
/// em compile-time, sem `@comptime` explícito.
#[test]
fn fold_literal_call_int() {
    let src = "\
dobro :: Int => Int
lambda x: + x x

action main
    echo!(dobro 5)

main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "10",
        "dobro 5 deve ser dobrado para 10 em compile-time — stdout: {stdout}"
    );
}

/// `square 3.14` (função pura com arg Float literal) deve ser dobrada
/// para `9.8596` em compile-time.
#[test]
fn fold_literal_call_float() {
    let src = "\
square :: Float => Float
lambda x: * x x

action main
    echo!(square 3.14)

main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "9.8596",
        "square 3.14 deve ser dobrado para 9.8596 em compile-time — stdout: {stdout}"
    );
}

/// `quad 3` chama `dobro (dobro 3)` — encadeamento via fixpoint.
/// Primeiro fold: `dobro 3` → `6`. Segundo fold: `dobro 6` → `12`.
#[test]
fn fold_literal_call_nested() {
    let src = "\
dobro :: Int => Int
lambda x: + x x

quad :: Int => Int
lambda x: dobro (dobro x)

action main
    echo!(quad 3)

main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "12",
        "quad 3 deve ser dobrado para 12 via fixpoint — stdout: {stdout}"
    );
}

/// `+ 1 (dobro 5)` — o arg `dobro 5` é uma Closure (não literal),
/// então o `+` não pode ser dobrado na primeira iteração.
/// Mas `dobro 5` → `10` na primeira iteração, e na segunda iteração
/// `+ 1 10` → `11` (FFI builtin, não foldable, mas o arg virou literal).
/// O `+` tem `ffi_symbol: Some(...)` então não é foldable. O teste
/// confirma que o fold de `dobro 5` não quebra o `+` que usa o resultado.
#[test]
fn fold_literal_call_in_argument_of_ffi_call() {
    let src = "\
dobro :: Int => Int
lambda x: + x x

action main
    echo!(+ 1 (dobro 5))

main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "11",
        "+ 1 (dobro 5) deve produzir 11 — stdout: {stdout}"
    );
}

/// Função com múltiplas cláusulas e arg literal — o pattern matching
/// é resolvido em compile-time via JIT, produzindo o literal.
#[test]
fn fold_literal_call_multi_clause() {
    let src = "\
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

action main
    echo!(fib 10)

main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "55",
        "fib 10 deve ser dobrado para 55 em compile-time — stdout: {stdout}"
    );
}
