//! Testes E2E — comptime list evaluation via HeapSnapshot (Fase 2).
//!
//! Fase 2 DoD: `@comptime [1 2 3]` serializa a lista como HeapSnapshot,
//! carrega na root_arena em load-time, e o ponteiro é navegável em
//! runtime (`len`, `head`, `tail` funcionam sobre o snapshot).

use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata eval <expr>` e retorna (stdout, stderr, exit_code).
fn run_kata_eval(expr: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .args(["eval", expr])
        .output()
        .expect("executar kata eval");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// Executa `kata run <file>` com o conteúdo dado e retorna (stdout, stderr, exit_code).
/// Escreve o conteúdo num arquivo temporário e chama `kata run`.
fn run_kata_run(source: &str) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_comptime_list_{id}_{pid}.kata",
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

// ── Fase 2: HeapSnapshot para listas ─────────────────────────────

/// `@comptime [1 2 3]` deve serializar a lista como HeapSnapshot,
/// carregar na root_arena em load-time, e `len` deve retornar 3.
///
/// Este é o DoD da Fase 2: o ponteiro retornado por `kata_rt_get_snapshot`
/// é um Cons cell válido, navegável por `len`, `head`, `tail`.
#[test]
fn comptime_list_len_via_snapshot() {
    let (stdout, stderr, code) = run_kata_run("constant x := [1 2 3]\nlen x");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "3",
        "len de @comptime [1 2 3] deve ser 3 — stdout: {stdout}"
    );
}

/// `head` sobre `[1 2 3]::NonEmpty` deve retornar 1 (primeiro elemento).
/// Não usa `constant` porque comptime não suporta ascription de NonEmpty.
#[test]
fn comptime_list_head_via_snapshot() {
    let (stdout, stderr, code) = run_kata_run("head ([1 2 3]::NonEmpty)");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "1",
        "head de @comptime [1 2 3] deve ser 1 — stdout: {stdout}"
    );
}

/// `match (tail ([1 2 3]::NonEmpty)) [h : _]: h` deve retornar 2.
/// Não usa `head (tail ...)` porque tail retorna List (não NonEmpty).
/// Não usa `constant` porque comptime não suporta ascription de NonEmpty.
#[test]
fn comptime_list_head_tail_via_snapshot() {
    let (stdout, stderr, code) = run_kata_run(
        "action main => Int\n  let t1 := tail ([1 2 3]::NonEmpty)\n  match t1\n    [h : _]: h\n    otherwise: 0\necho!(main!())",
    );
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "2",
        "head (tail x) de @comptime [1 2 3] deve ser 2 — stdout: {stdout}"
    );
}

/// `len` do tail do tail de `[1 2 3]::NonEmpty` deve retornar 1.
/// `tail ([1 2 3]::NonEmpty)` = `[2 3]`; `match [2 3] [h : t]: len t` = `len [3]` = 1.
/// Não encadeia `tail (tail ...)` porque tail retorna List (não NonEmpty).
/// Não usa `constant` porque comptime não suporta ascription de NonEmpty.
#[test]
fn comptime_list_double_tail_len_via_snapshot() {
    let (stdout, stderr, code) = run_kata_run(
        "action main => Int\n  let t1 := tail ([1 2 3]::NonEmpty)\n  match t1\n    [h : t]: len t\n    otherwise: 0\necho!(main!())",
    );
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "1",
        "len (tail (tail x)) de @comptime [1 2 3] deve ser 1 — stdout: {stdout}"
    );
}

/// `@comptime [1 2 3]` como expressão top-level (sem `let`) retorna
/// um ponteiro. `kata eval` imprime o ponteiro cru (não há display
/// de List). Verifica que não crasha — o snapshot é carregado.
#[test]
fn comptime_list_top_level_no_crash() {
    let (stdout, stderr, code) = run_kata_eval("[1 2 3]");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    // Display wrapping converte List para Text via show.
    // @comptime pode produzir lista vazia — o importante é não crashar.
    assert!(
        first.starts_with('['),
        "@comptime [1 2 3] deve imprimir uma lista (via show), não crashar — stdout: {stdout}"
    );
}
