//! Testes E2E — `@cache` estratégias de eviction (Fio 12, Fase 5).
//!
//! Estes testes exercitam as diferentes estratégias de cache:
//! - FIFO (First In First Out)
//! - MRU (Most Recently Used)
//! - LFU (Least Frequently Used)

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
        "kata_cache_strategies_e2e_{id}_{pid}.kata",
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

// ── FIFO ──────────────────────────────────────────────────────────

/// `@cache{strategy: "FIFO"}` caso básico — mesmo resultado que LRU
/// quando não há eviction.
#[test]
fn cache_fifo_basic() {
    let src = "\
@cache{strategy: \"FIFO\"}
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "10", "dobro 5 deve ser 10 — stdout: {stdout}");
}

/// FIFO eviction: capacity=3, insere 4 keys. A primeira inserida (k1)
/// deve ser evicta, não a menos recentemente acessada.
#[test]
fn cache_fifo_eviction() {
    let src = "\
@cache{strategy: \"FIFO\", capacity: 3}
f :: Int => Int
lambda x: x

action main
    echo!(f 1)
    echo!(f 2)
    echo!(f 3)
    echo!(f 1)
    echo!(f 4)
    echo!(f 1)
main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 6, "deve imprimir 6 linhas — stdout: {stdout}");
    // f 1 → 1 (miss, insert k1)
    // f 2 → 2 (miss, insert k2)
    // f 3 → 3 (miss, insert k3)
    // f 1 → 1 (HIT — k1 ainda está, só 3 entradas)
    // f 4 → 4 (miss, evict k1 por FIFO, insert k4)
    // f 1 → 1 (miss — k1 foi evicta, mas f 1 = 1)
    assert_eq!(lines[0], "1", "f 1 deve ser 1");
    assert_eq!(lines[3], "1", "f 1 (hit) deve ser 1");
    assert_eq!(lines[4], "4", "f 4 deve ser 4");
    assert_eq!(lines[5], "1", "f 1 (após eviction) deve ser 1");
}

/// FIFO não promove por acesso: lookup não afeta eviction.
/// capacity=2, insere k1, k2. Acessa k1. Insere k3.
/// FIFO evicta k1 (primeira inserida), não k2.
#[test]
fn cache_fifo_no_promote_on_lookup() {
    let src = "\
@cache{strategy: \"FIFO\", capacity: 2}
f :: Int => Int
lambda x: x

action main
    echo!(f 1)
    echo!(f 2)
    echo!(f 1)
    echo!(f 3)
    echo!(f 2)
main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 5, "deve imprimir 5 linhas — stdout: {stdout}");
    // f 1 → 1 (miss, insert k1, order=1)
    // f 2 → 2 (miss, insert k2, order=2)
    // f 1 → 1 (HIT — k1 ainda está, FIFO não promove)
    // f 3 → 3 (miss, evict k1 por FIFO, insert k3)
    // f 2 → 2 (HIT — k2 sobreviveu, era order=2 > k1 order=1)
    assert_eq!(lines[2], "1", "f 1 (hit) deve ser 1");
    assert_eq!(lines[3], "3", "f 3 deve ser 3");
    assert_eq!(lines[4], "2", "f 2 (hit, sobreviveu) deve ser 2");
}

// ── MRU ────────────────────────────────────────────────────────────

/// `@cache{strategy: "MRU"}` caso básico.
#[test]
fn cache_mru_basic() {
    let src = "\
@cache{strategy: \"MRU\"}
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "10", "dobro 5 deve ser 10 — stdout: {stdout}");
}

/// MRU eviction: capacity=3, insere k1, k2, k3. Acessa k1 (vira MRU).
/// Insere k4. MRU evicta k1 (mais recentemente acessada).
#[test]
fn cache_mru_eviction() {
    let src = "\
@cache{strategy: \"MRU\", capacity: 3}
f :: Int => Int
lambda x: x

action main
    echo!(f 1)
    echo!(f 2)
    echo!(f 3)
    echo!(f 1)
    echo!(f 4)
    echo!(f 1)
    echo!(f 2)
main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 7, "deve imprimir 7 linhas — stdout: {stdout}");
    // f 1 → 1 (miss, insert. last_access: k1=1)
    // f 2 → 2 (miss, insert. last_access: k2=2)
    // f 3 → 3 (miss, insert. last_access: k3=3)
    // f 1 → 1 (HIT. last_access: k1=4 — agora é MRU!)
    // f 4 → 4 (miss, evict k1 (maior last_access=4), insert k4. last_access: k4=5)
    // f 1 → 1 (miss — k1 foi evicta)
    // f 2 → 2 (HIT — k2 sobreviveu, last_access=2 < k3=3 < k4=5)
    assert_eq!(lines[3], "1", "f 1 (hit) deve ser 1");
    assert_eq!(lines[4], "4", "f 4 deve ser 4");
    assert_eq!(lines[5], "1", "f 1 (após eviction) deve ser 1");
    assert_eq!(lines[6], "2", "f 2 (hit, sobreviveu) deve ser 2");
}

// ── LFU ────────────────────────────────────────────────────────────

/// `@cache{strategy: "LFU"}` caso básico.
#[test]
fn cache_lfu_basic() {
    let src = "\
@cache{strategy: \"LFU\"}
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "10", "dobro 5 deve ser 10 — stdout: {stdout}");
}

/// LFU eviction: capacity=3, insere k1, k2, k3. Acessa k1 2x, k2 1x.
/// Insere k4. LFU evicta k3 (count=1, menor).
#[test]
fn cache_lfu_eviction() {
    let src = "\
@cache{strategy: \"LFU\", capacity: 3}
f :: Int => Int
lambda x: x

action main
    echo!(f 1)
    echo!(f 2)
    echo!(f 3)
    echo!(f 1)
    echo!(f 1)
    echo!(f 2)
    echo!(f 4)
    echo!(f 3)
    echo!(f 2)
    echo!(f 1)
main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        10,
        "deve imprimir 10 linhas — stdout: {stdout}"
    );
    // f 1 → miss, count: k1=1
    // f 2 → miss, count: k2=1
    // f 3 → miss, count: k3=1
    // f 1 → HIT, count: k1=2
    // f 1 → HIT, count: k1=3
    // f 2 → HIT, count: k2=2
    // f 4 → miss, evict k3 (count=1, menor), insert k4. count: k4=1
    // f 3 → miss (k3 foi evicta)
    // f 2 → HIT (k2 sobreviveu, count=2)
    // f 1 → HIT (k1 sobreviveu, count=3)
    assert_eq!(lines[6], "4", "f 4 deve ser 4");
    assert_eq!(lines[8], "2", "f 2 (hit, sobreviveu) deve ser 2");
    assert_eq!(lines[9], "1", "f 1 (hit, sobreviveu) deve ser 1");
}

/// LFU new-key penalty: capacity=2, insere k1, k2. Acessa k1 5x.
/// Insere k3 (count=1). Insere k4 (count=1). LFU evicta k3 (count=1).
#[test]
fn cache_lfu_new_key_penalty() {
    let src = "\
@cache{strategy: \"LFU\", capacity: 2}
f :: Int => Int
lambda x: x

action main
    echo!(f 1)
    echo!(f 2)
    echo!(f 1)
    echo!(f 1)
    echo!(f 1)
    echo!(f 1)
    echo!(f 1)
    echo!(f 3)
    echo!(f 4)
    echo!(f 3)
main!()";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        10,
        "deve imprimir 10 linhas — stdout: {stdout}"
    );
    // f 1 → miss, count: k1=1
    // f 2 → miss, count: k2=1
    // f 1 x5 → HIT, count: k1=6
    // f 3 → miss, evict k2 (count=1 < k1=6), insert k3. count: k3=1
    // f 4 → miss, evict k3 (count=1, empatou com... só k1=6 e k3=1), insert k4
    // f 3 → miss (k3 foi evicta)
    assert_eq!(lines[7], "3", "f 3 deve ser 3");
    assert_eq!(lines[9], "3", "f 3 (após eviction) deve ser 3");
}
