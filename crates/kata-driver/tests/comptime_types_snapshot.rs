//! Testes E2E — comptime Text, Struct, Tuple, Sum via HeapSnapshot (Fase 2).
//!
//! Testa serialização de tipos complexos (Text, Struct, Tuple, Sum) via
//! HeapSnapshot, carregados na root_arena em load-time e navegáveis em runtime.

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
        "kata_comptime_types_{id}_{pid}.kata",
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

// ── Fase 2: Text via snapshot ─────────────────────────────────────

/// `@comptime "hello"` top-level deve imprimir `hello` — a string
/// é serializada para a appended section e o codegen faz `load(ptr+0)`
/// para obter o ponteiro da C string.
#[test]
fn comptime_text_top_level() {
    let (stdout, stderr, code) = run_kata_eval("\"hello\"");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "hello",
        "@comptime \"hello\" deve imprimir hello — stdout: {stdout}"
    );
}

/// `@comptime let x := "hello"` + `len x` → 5. Exercita Text no
/// snapshot + `kata_rt_string_len` (SMI-tagged).
#[test]
fn comptime_text_len() {
    let (stdout, stderr, code) = run_kata_run("constant x := \"hello\"\nlen x");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "5",
        "len de @comptime \"hello\" deve ser 5 — stdout: {stdout}"
    );
}

/// `@comptime ["a" "b" "c"]` + `len x` → 3. Lista de Text onde cada
/// head é um ponteiro para a appended section.
#[test]
fn comptime_list_of_text_len() {
    let (stdout, stderr, code) = run_kata_run("constant x := [\"a\" \"b\" \"c\"]\nlen x");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "3",
        "len de @comptime [\"a\" \"b\" \"c\"] deve ser 3 — stdout: {stdout}"
    );
}

/// `len (head (["hello" "world"]::NonEmpty))` → 5.
/// Não usa `constant` porque comptime não suporta ascription de NonEmpty.
#[test]
fn comptime_list_of_text_head_len() {
    let (stdout, stderr, code) = run_kata_run("len (head ([\"hello\" \"world\"]::NonEmpty))");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "5",
        "len (head x) de [\"hello\" \"world\"] deve ser 5 — stdout: {stdout}"
    );
}

// ── Fase 2: Struct via snapshot ───────────────────────────────────

/// `constant p := Pessoa "Alice" 30` + `p.idade` → 30.
/// Struct com campo Text serializada via snapshot, acesso por campo.
#[test]
fn comptime_struct_field_access() {
    let src = "data Pessoa (nome::Text idade::Int)\nconstant p := Pessoa \"Alice\" 30\np.idade";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "30",
        "p.idade de @comptime Pessoa deve ser 30 — stdout: {stdout}"
    );
}

/// `len p.nome` onde `p := @comptime Pessoa "Alice" 30` → 5.
/// Campo Text em Struct acessado e navegado (len desreferencia a string).
#[test]
fn comptime_struct_text_field_len() {
    let src = "data Pessoa (nome::Text idade::Int)\nconstant p := Pessoa \"Alice\" 30\nlen p.nome";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "5",
        "len p.nome de @comptime Pessoa deve ser 5 — stdout: {stdout}"
    );
}

// ── Fase 2: Tuple via snapshot ────────────────────────────────────

/// `@comptime (1, 2, 3)` + `x.0` → 1. Tuple sem regressão.
#[test]
fn comptime_tuple_index_access() {
    let (stdout, stderr, code) = run_kata_run("constant x := (1, 2, 3)\nx.0");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "1",
        "x.0 de @comptime (1, 2, 3) deve ser 1 — stdout: {stdout}"
    );
}

// ── Fase 2: Sum via snapshot ───────────────────────────────────────

/// `@comptime Ok 42` + match → 42. Sum com payload Int
/// (SMI) serializado via snapshot, desempacotado por match.
#[test]
fn comptime_sum_int_match() {
    let src = "constant r := Ok 42\nmatch r\n    Ok v: v\n    Err e: 0";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "42",
        "match @comptime Ok 42 deve produzir 42 — stdout: {stdout}"
    );
}

/// `@comptime Err "fail"` + match → `fail`. Sum com payload
/// Text. O payload é copiado como ponteiro cru no serializer atual;
/// funciona porque a arena comptime sobrevive até o fim do processo.
#[test]
fn comptime_sum_text_match() {
    let src = "constant r := Err \"fail\"\nmatch r\n    Ok v: v\n    Err e: e";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "fail",
        "match @comptime Err \"fail\" deve imprimir fail — stdout: {stdout}"
    );
}

/// `len e` no match de `@comptime Err "fail"` → 4.
/// Exercita Text como payload de Sum acessado por `len`.
#[test]
fn comptime_sum_text_match_len() {
    let src = "constant r := Err \"fail\"\nmatch r\n    Ok v: v\n    Err e: len e";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "4",
        "len e de @comptime Err \"fail\" deve ser 4 — stdout: {stdout}"
    );
}
