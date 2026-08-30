//! E2E — `@cache` no interpretador (deferred da sessão de escopo-plano).
//!
//! Responsabilidade: cravar que o INTERP respeita `@cache` igual ao JIT:
//! hit retorna sem reexecutar o body da função. O interp despachava
//! `call_typed_clauses` sem consultar `cache_spec` — memoização
//! inexistente (fib 35 travava; cache.kata era o único divergente do
//! oráculo interp-vs-JIT).
//!
//! Oráculo de comportamento: side-effect DENTRO do body (`echo!`) —
//! só o INTERP (o codegen do JIT não suporta ActionCall em função
//! pura; o `@log{enter}` dispara no wrapper antes do lookup, então
//! não conta body). Testes de valor rodam nos DOIS backends.
//!
//! Nota: último statement da action é ret-directed (hint Unit) —
//! chamadas de função pura vão sempre dentro de `echo!`.

use std::process::Command;

/// Roda `kata run [--interp] <src>` e retorna (stdout, stderr, code).
fn run_kata(path: &str, interp: bool) -> (String, String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kata"));
    if interp {
        cmd.args(["run", "--interp", path]);
    } else {
        cmd.args(["run", path]);
    }
    let out = cmd.output().expect("kata run deve executar");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// Escreve source num .kata temporário de nome ÚNICO e retorna o path.
/// (Testes rodam em paralelo — nome compartilhado = race condition.)
fn write_temp(name: &str, src: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kata_cache_interp_e2e_{name}_{id}_{}.kata",
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().to_string()
}

/// Roda no INTERP e exige (stdout, code).
fn run_interp(src: &str, expected: &str, label: &str) {
    let path = write_temp("interp_case", src);
    let (out, err, code) = run_kata(&path, true);
    assert_eq!(code, 0, "interp deve exit 0 — stderr: {err}");
    assert_eq!(out, expected, "interp: {label}");
}

/// Roda nos DOIS backends e exige o mesmo stdout.
fn assert_both(src: &str, expected: &str) {
    let path = write_temp("case", src);
    let (out_i, err_i, code_i) = run_kata(&path, true);
    assert_eq!(
        code_i, 0,
        "interp deve exit 0 — stderr: {err_i}\nstdout: {out_i}"
    );
    assert_eq!(out_i, expected, "interp: stdout divergente");
    let (out_j, err_j, code_j) = run_kata(&path, false);
    assert_eq!(
        code_j, 0,
        "JIT deve exit 0 — stderr: {err_j}\nstdout: {out_j}"
    );
    assert_eq!(out_j, expected, "JIT: stdout divergente");
}

// ── Comportamento: hit não reexecuta o body (INTERP) ───────────

/// `dobro` com `@cache` + `echo!` no body: chamado 2× com mesmo arg.
/// Cache hit na 2ª → body não roda de novo → 1 "executou".
/// RED hoje no interp: reexecuta → 2 "executou".
#[test]
fn cache_hit_nao_reexecuta_body() {
    let source = r#"@cache{strategy: "LRU"}
dobro :: Int => Int
lambda n:
    echo!("executou")
    * n 2

action main
    echo!(dobro 5)
    echo!(dobro 5)
main!()"#;
    run_interp(&source, "executou\n10\n10\n", "hit pula body");
}

/// Sem `@cache`: body roda toda vez → 2 "executou" (controle).
#[test]
fn sem_cache_reexecuta() {
    let source = r#"dobro :: Int => Int
lambda n:
    echo!("executou")
    * n 2

action main
    echo!(dobro 5)
    echo!(dobro 5)
main!()"#;
    run_interp(
        &source,
        "executou\n10\nexecutou\n10\n",
        "sem cache = 2× body",
    );
}

/// Float na key: `square 3.14` duas vezes — hit não reexecuta.
#[test]
fn cache_key_float() {
    let source = r#"@cache{strategy: "LRU"}
square :: Float => Float
lambda x:
    echo!("executou")
    * x x

action main
    echo!(square 3.14)
    echo!(square 3.14)
main!()"#;
    run_interp(&source, "executou\n9.8596\n9.8596\n", "hit em Float key");
}

/// Text key: strings diferentes = keys diferentes (por conteúdo).
#[test]
fn cache_key_text() {
    let source = r#"@cache{strategy: "LRU"}
tag :: Text => Text
lambda s:
    echo!("executou")
    s

action main
    echo!(tag "aaa")
    echo!(tag "aaa")
    echo!(tag "bb")
main!()"#;
    run_interp(
        &source,
        "executou\naaa\naaa\nexecutou\nbb\n",
        "hit em Text key",
    );
}

/// Overloads: cada overload tem cache próprio (fn_id distinto).
#[test]
fn overloads_cache_independente() {
    let source = r#"@cache{strategy: "LRU"}
f :: Int => Int
lambda n:
    echo!("exec_int")
    + n 1

@cache{strategy: "LRU"}
f :: Float => Float
lambda x:
    echo!("exec_float")
    * x 2.0

action main
    echo!(f 1)
    echo!(f 1)
    echo!(f 1.5)
    echo!(f 1.5)
main!()"#;
    run_interp(
        &source,
        "exec_int\n2\n2\nexec_float\n3.0\n3.0\n",
        "caches independentes por overload",
    );
}

// ── Valores: hit retorna o valor cacheado (AMBOS) ─────────────

/// `fib 35` memoizado — instantâneo nos dois backends.
/// RED hoje: interp trava (O(φⁿ) sem memoização).
#[test]
fn fib_35_memoizado_instantaneo() {
    let source = r#"@cache{strategy: "LRU"}
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

action main
    echo!(fib 35)
main!()"#;
    assert_both(&source, "9227465\n");
}

/// As 4 estratégias de eviction rodam nos dois backends (mesma API
/// do runtime). fib 10 = 55 com capacity apertada.
#[test]
fn quatro_estrategias_eviction() {
    for s in ["LRU", "FIFO", "MRU", "LFU"] {
        let source = format!(
            r#"@cache{{strategy: "{s}", capacity: 2}}
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

action main
    echo!(fib 10)
main!()"#
        );
        let path = write_temp("estrategia", &source);
        for (interp, name) in [(true, "interp"), (false, "JIT")] {
            let (out, err, code) = run_kata(&path, interp);
            assert_eq!(code, 0, "{name}: {s} deve exit 0 — {err}");
            assert_eq!(out, "55\n", "{name}: {s} fib 10 = 55");
        }
    }
}

/// Key composto: dois args Int. sum 1 2 ≠ sum 2 1 (evita colisão).
#[test]
fn cache_key_multi_arg() {
    let source = r#"@cache{strategy: "LRU"}
sum :: Int Int => Int
lambda a b: - a b

action main
    echo!(sum 1 2)
    echo!(sum 2 1)
    echo!(sum 1 2)
main!()"#;
    assert_both(&source, "-1\n1\n-1\n");
}
