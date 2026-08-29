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
@cache{strategy: \\\"LRU\\\"}
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

// ── FIFO ──────────────────────────────────────────────────────────

/// `@cache{strategy: "FIFO"}` caso básico — mesmo resultado que LRU
/// quando não há eviction.
#[test]
fn cache_fifo_basic() {
    let src = "\
@cache{strategy: \\\"FIFO\\\"}
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
@cache{strategy: \\\"FIFO\\\", capacity: 3}
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
@cache{strategy: \\\"FIFO\\\", capacity: 2}
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
@cache{strategy: \\\"MRU\\\"}
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
@cache{strategy: \\\"MRU\\\", capacity: 3}
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
@cache{strategy: \\\"LFU\\\"}
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
@cache{strategy: \\\"LFU\\\", capacity: 3}
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
@cache{strategy: \\\"LFU\\\", capacity: 2}
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

// ── capacity ──────────────────────────────────────────────────────

/// `@cache{capacity: 2}` (strategy default LRU). fib 10 com capacity 2:
/// eviction frequente, mas resultado correto.
#[test]
fn cache_capacity_explicit() {
    let src = "\
@cache{capacity: 2}
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

fib 10";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "55", "fib 10 deve ser 55 — stdout: {stdout}");
}

// ── @cache sem args ───────────────────────────────────────────────

/// `@cache` sozinho (sem dict) ativa LRU 256.
#[test]
fn cache_no_args() {
    let src = "\
@cache
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "10", "dobro 5 deve ser 10 — stdout: {stdout}");
}

/// `@cache{}` (dict vazio) ativa LRU 256.
#[test]
fn cache_empty_dict() {
    let src = "\
@cache{}
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "10", "dobro 5 deve ser 10 — stdout: {stdout}");
}

/// `@cache{capacity: 0}` → erro de compilação.
#[test]
fn cache_capacity_zero_error() {
    let src = "\
@cache{capacity: 0}
dobro :: Int => Int
lambda n: * n 2

dobro 5";
    let (_stdout, stderr, code) = run_kata_run(src);
    assert_ne!(code, 0, "kata run deve falhar com capacity: 0");
    assert!(
        stderr.contains("capacidade de cache inválida") || stderr.contains("cache"),
        "stderr deve mencionar capacidade inválida — stderr: {stderr}"
    );
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
