//! Testes E2E — `@cache{strategy: "LRU"}` (Fio 12, Fase 5).
//!
//! PRD-fio12-comptime.md §3.4: `@cache` anota a definição da função. O codegen
//! emite cache lookup no prólogo e insert no epílogo. O cache é fiber-local
//! (TLS HashMap, LRU, 256 entradas).
//!
//! Estes testes exercitam:
//! - Caso básico (cláusula única, Int => Int) — DoD da Fase 5
//! - Múltiplas cláusulas + @cache (ponto 2 do handoff)
//! - Memoização efetiva (fib 35 seria intratável sem cache)

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
        "kata_cache_e2e_{id}_{pid}.kata",
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

// ── DoD Fase 5: caso básico, cláusula única ───────────────────────

/// `dobro 5` com `@cache` → 10. Caso básico Int => Int.
#[test]
fn cache_basic_single_clause() {
    let src = "\
@cache{strategy: \"LRU\"}
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "10", "dobro 5 deve ser 10 — stdout: {stdout}");
}

/// `dobro 5` chamado 3x — cache hit na 2ª e 3ª chamada.
/// O resultado deve ser o mesmo (10).
#[test]
fn cache_hit_returns_same_value() {
    let src = "\
@cache{strategy: \"LRU\"}
dobro :: Int => Int
lambda n: * n 2

action main
    let a := dobro 5
    let b := dobro 5
    let c := dobro 5
    echo!(a)
    echo!(b)
    echo!(c)

main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "deve imprimir 3 linhas — stdout: {stdout}");
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(*line, "10", "linha {i} deve ser 10 — stdout: {stdout}");
    }
}

// ── Ponto 2: múltiplas cláusulas + @cache ──────────────────────────

/// `fatorial 5` com múltiplas cláusulas + `@cache` → 120.
/// O codegen de `lower_clause_chain` já faz `jump(epilogue)` quando
/// `epilogue_block` está definido, então o `cache_insert` deve executar.
#[test]
fn cache_multi_clause_fatorial() {
    let src = "\
@cache{strategy: \"LRU\"}
fatorial :: Int => Int
lambda 0: 1
lambda n: * n (fatorial (- n 1))

fatorial 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "120", "fatorial 5 deve ser 120 — stdout: {stdout}");
}

/// `fib 35` com múltiplas cláusulas + `@cache` → 9227465.
/// Sem memoização, fib 35 faz ~14M chamadas recursivas — seria intratável.
/// Se o cache funciona, é instantâneo. Este teste prova a memoização
/// efetiva com múltiplas cláusulas: cada subproblema (fib k) só computa uma vez.
#[test]
fn cache_multi_clause_fib35_memoization_proof() {
    let src = "\
@cache{strategy: \"LRU\"}
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

fib 35";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "9227465",
        "fib 35 com cache deve ser 9227465 — stdout: {stdout}"
    );
}

// ── Ponto 2: cláusula única com guard ─────────────────────────────

/// `abs (-5)` com cláusula única + guard + `@cache` → 5.
/// Exercita `lower_guards` + `@cache` — o guard body faz `jump(cont_block)`,
/// que depois faz `jump(epilogue_block)`.
#[test]
fn cache_single_clause_with_guard() {
    let src = "\
@cache{strategy: \"LRU\"}
abs :: Int => Int
lambda n:
    >= n 0: n
    otherwise: - 0 n

abs (-5)";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "5", "abs (-5) deve ser 5 — stdout: {stdout}");
}

// ── Ponto 3: tail call + @cache ────────────────────────────────────

/// `fat_tail 5 1` com `@cache` e tail call recursivo → 120.
/// Sem `no_tail_calls`, o `return_call` pula o epílogo onde `cache_insert`
/// vive. Com `no_tail_calls=true` (setado quando `cache_spec.is_some()`),
/// o codegen emite `call` normal, o resultado volta como SSA value, e o
/// epílogo executa `cache_insert`.
#[test]
fn cache_tail_call_recursive() {
    let src = "\
@cache{strategy: \"LRU\"}
fat_tail :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat_tail (- n 1) (* n acc)

fat_tail 5 1";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "120", "fat_tail 5 1 deve ser 120 — stdout: {stdout}");
}

/// `fib_tail 30 0 1` com `@cache` e tail call recursivo.
/// Verifica que o cache não quebra a recursão tail-recursive.
/// fib_tail 30 0 1 = 832040.
#[test]
fn cache_tail_call_fib() {
    let src = "\
@cache{strategy: \"LRU\"}
fib_tail :: Int Int Int => Int
lambda 0 a b: a
lambda 1 a b: b
lambda n a b: fib_tail (- n 1) b (+ a b)

fib_tail 30 0 1";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "832040",
        "fib_tail 30 0 1 deve ser 832040 — stdout: {stdout}"
    );
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

action main => Unit
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
