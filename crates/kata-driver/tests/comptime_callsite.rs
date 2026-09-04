//! Testes E2E — comptime em call-site (bodies de actions) (Fase 3).
//!
//! Testa que `@comptime` funciona dentro de bodies de actions,
//! incluindo expressões, bindings, dataflow, params, pure function
//! calls, e pipes.

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
        "kata_comptime_callsite_{id}_{pid}.kata",
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

// ── Fase 3: @comptime em call-site (bodies de actions) ───────────

/// `@comptime + 1 2` dentro de body de action → 3.
/// O @comptime é avaliado em compile-time e substituído por literal 3.
#[test]
fn comptime_callsite_expr_in_body() {
    let src = "action main => Int\n    + 1 2\nmain!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "3",
        "@comptime + 1 2 em body deve produzir 3 — stdout: {stdout}"
    );
}

/// `@comptime let x := + 1 2` dentro de body → binding com literal 3.
/// Depois `echo!(x)` imprime 3.
#[test]
fn comptime_callsite_let_in_body() {
    let src = "action main\n    let x := + 1 2\n    echo!(x)\nmain!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "3",
        "constant x := + 1 2 em body deve produzir 3 — stdout: {stdout}"
    );
}

/// `@comptime let x := 10` seguido de `@comptime + x 5` em body → 15.
/// Exercita dataflow: o binding `x` é comptime-available após o primeiro
/// @comptime let, e o segundo @comptime o referencia.
#[test]
fn comptime_callsite_dataflow_binding() {
    let src = "action main\n    let x := 10\n    let y := + x 5\n    echo!(y)\nmain!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "15",
        "@comptime dataflow + x 5 deve produzir 15 — stdout: {stdout}"
    );
}

/// `+ x 5` onde `x` é param de action → 15 (runtime, sem @comptime).
/// Antes testava que @comptime rejeitava params; agora @comptime foi removido
/// e `+ x 5` é avaliado em runtime normalmente.
#[test]
fn comptime_callsite_param_not_comptime() {
    let src = "action foo (x::Int) => Int\n    + x 5\nfoo!(10)";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "15",
        "+ x 5 com x=10 deve produzir 15 — stdout: {stdout}"
    );
}

/// `@comptime echo!("msg")` — `@comptime` foi removido, erro de parser.
#[test]
fn comptime_callsite_impure_action_call() {
    let src = "action main\n    @comptime echo!(\"msg\")\nmain!()";
    let (_stdout, _stderr, code) = run_kata_run(src);
    assert_ne!(
        code, 0,
        "kata run deve falhar (@comptime removido) — code: {code}"
    );
}

/// `@comptime fib 10` dentro de body → 55.
/// Exercita chamada de função pura com literal em call-site.
#[test]
fn comptime_callsite_fib_in_body() {
    let src = "\
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n:
    + (fib (- n 1)) (fib (- n 2))

action main
    let r := fib 10
    echo!(r)

main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "55",
        "@comptime fib 10 em body deve produzir 55 — stdout: {stdout}"
    );
}

/// `@comptime 5 |> (+ 1 _)` — pipe é runtime, não comptime.
/// `@comptime 5` avalia para 5, depois `|> (+ 1 _)` é runtime: 5 + 1 = 6.
#[test]
fn comptime_callsite_pipe_is_runtime() {
    let src = "action main => Int\n    5 |> + 1 _\nmain!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "6",
        "@comptime 5 |> + 1 _ deve imprimir 6 (pipe é runtime) — stdout: {stdout}"
    );
}
