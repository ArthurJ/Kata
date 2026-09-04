//! Testes E2E — `@cache` + TCO (Tail Call Optimization) (Fio 12, Fase 5).
//!
//! Estes testes exercitam o wrapper/inner split que preserva TCO com `@cache`:
//! - TCO com n grande (1M) — sem TCO, stack overflow
//! - Função mista (tail + non-tail)
//! - `@cache` + `@timer` + TCO

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
        "kata_cache_tco_e2e_{id}_{pid}.kata",
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

// ── Wrapper/inner split: TCO preservado com @cache ──────────────────

/// `fat_tail 1000000 1` com `@cache` — deve completar sem stack overflow.
/// Com o wrapper/inner split, o inner faz TCO (stack O(1)) e o wrapper
/// tem 1 frame extra. Sem o split, `no_tail_calls=true` faria stack O(n).
/// n=1M é a prova de TCO — sem TCO, 1M frames causam SIGSEGV.
#[test]
fn cache_tco_large_n() {
    let src = "\
@cache{strategy: \"LRU\"}
count_down :: Int Int => Int
lambda 0 acc: acc
lambda n acc: count_down (- n 1) acc

count_down 1000000 1";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(
        code, 0,
        "kata run deve exit 0 (TCO deve evitar stack overflow) — stderr: {stderr}"
    );
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "1",
        "count_down 1000000 1 deve ser 1 — stdout: {stdout}"
    );
}

/// Função mista (tail + non-tail) com `@cache`.
/// `f(15)` → wrapper → miss → inner → f(14) (tail → inner, TCO) → ...
/// → f(9) → inner → + (f(8)) 1 (non-tail → wrapper, cache) → ...
/// Resultado: 9 (f(15)=f(14)=...=f(10)=f(9)=9, f(9)=f(8)+1=8+1=9)
#[test]
fn cache_mixed_tail_nontail() {
    let src = "\
@cache
f :: Int => Int
lambda 0: 0
lambda n:
    >= n 10: f (- n 1)
    otherwise: + (f (- n 1)) 1

f 15";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "9", "f 15 deve ser 9 — stdout: {stdout}");
}

/// `@cache` + `@timer` + TCO: completa, cachear, e medir tempo.
#[test]
fn cache_timer_tco() {
    let src = "\
@cache{strategy: \"LRU\"}
@timer{topic: \"perfil\"}
count_down :: Int Int => Int
lambda 0 acc: acc
lambda n acc: count_down (- n 1) acc

action chamar => Int
    let r := count_down 100000 0
    r

action consumir => Int
    let msg := log_recv!(\"perfil\")
    echo!(msg)
    0

fork!(chamar, ())
consumir!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(
        code, 0,
        "kata run deve exit 0 (TCO + cache + timer) — stderr: {stderr}"
    );
    // Deve publicar no tópico "perfil" com delta > 0.
    assert!(
        stdout.contains("count_down:") && stdout.contains("ns"),
        "deve imprimir 'fat_tail: ...ns' — stdout: {stdout} | stderr: {stderr}"
    );
}
