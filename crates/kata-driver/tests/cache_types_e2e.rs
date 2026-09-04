//! Testes E2E — `@cache` cache key para todos os tipos (Fio 12, Fase 5).
//!
//! Estes testes exercitam a serialização da cache key para diferentes tipos:
//! - Float (F64 bitcast)
//! - Text (cópia de bytes)
//! - Struct (serialização campo a campo)
//! - List (percorrendo cons cells)

use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata run <file>` com o conteúdo dado e retorna (stdout, stderr, exit_code).
fn run_kata_run(source: &str) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_cache_types_e2e_{id}_{pid}.kata",
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

// ── Ponto 4: cache key para todos os tipos ───────────────────────────

/// `square 3.14` com `@cache` → 9.8596.
/// Float é F64 no codegen. O hit block faz bitcast I64→F64 do valor
/// em cache, e o epílogo faz bitcast F64→I64 antes de cache_insert.
#[test]
fn cache_float_type() {
    let src = "\
@cache{strategy: \"LRU\"}
square :: Float => Float
lambda x: * x x

square 3.14";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert!(
        first.starts_with("9.8596"),
        "square 3.14 deve ser ~9.8596 — stdout: {stdout}"
    );
}

/// `greet "world"` com `@cache` → "world".
/// Text é ponteiro para C string. A serialização copia os bytes
/// da string (len + bytes), não o ponteiro.
#[test]
fn cache_text_type() {
    let src = "\
@cache{strategy: \"LRU\"}
greet :: Text => Text
lambda name: name

greet \"world\"";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "world",
        "greet world deve ser world — stdout: {stdout}"
    );
}

/// `idade_pessoa p` com `@cache` → 30.
/// Struct é serializada campo a campo. O type descriptor descreve
/// n_fields + tipo de cada field. O runtime lê cada field por offset.
#[test]
fn cache_struct_type() {
    let src = "\
data Pessoa (nome::Text idade::Int)

@cache{strategy: \"LRU\"}
idade_pessoa :: Pessoa => Int
lambda p: p.idade

action main => Unit
    let p := Pessoa \"Alice\" 30
    echo!(idade_pessoa p)
    echo!(idade_pessoa p)
main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "deve imprimir 2 linhas — stdout: {stdout}");
    assert_eq!(
        lines[0], "30",
        "primeira chamada deve ser 30 — stdout: {stdout}"
    );
    assert_eq!(
        lines[1], "30",
        "segunda chamada (cache hit) deve ser 30 — stdout: {stdout}"
    );
}

/// `head_or_zero [42 1 2]` com `@cache` → 42.
/// List é serializada percorrendo cons cells. O type descriptor
/// descreve o tipo do elemento. O runtime caminha head/tail de cada cell.
#[test]
fn cache_list_type() {
    let src = "\
@cache{strategy: \"LRU\"}
head_or_zero :: List::Int => Int
lambda []: 0
lambda [h : t]: h

action main
    echo!(head_or_zero [42 1 2])
    echo!(head_or_zero [42 1 2])
main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "deve imprimir 2 linhas — stdout: {stdout}");
    assert_eq!(
        lines[0], "42",
        "primeira chamada deve ser 42 — stdout: {stdout}"
    );
    assert_eq!(
        lines[1], "42",
        "segunda chamada (cache hit) deve ser 42 — stdout: {stdout}"
    );
}
