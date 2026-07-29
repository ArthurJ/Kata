//! Testes E2E — `@comptime` (Fio 12, Fases 1 e 2).
//!
//! PRD-fio12-comptime.md: `@comptime` avalia expressões em compile-time
//! via JIT-and-execute, substituindo o nó `Comptime` por um literal na TAST.
//!
//! Fase 1 DoD: `@comptime let x := + 1 2` gera `x = 3` — a expressão
//! `+ 1 2` é avaliada em compile-time e substituída por `IntLit "3"`.
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
        "kata_comptime_e2e_{id}_{pid}.kata",
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

// ── DoD Fase 1: @comptime + 1 2 → 3 ────────────────────────────────

/// `@comptime + 1 2` deve avaliar `+ 1 2` em compile-time e substituir
/// por `3`. O programa imprime `3`.
#[test]
fn comptime_add_two_ints() {
    let (stdout, stderr, code) = run_kata_eval("@comptime + 1 2");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "3",
        "@comptime + 1 2 deve produzir 3 — stdout: {stdout}"
    );
}

// ── SMI decoding: resultado é 30, não 61 (SMI de 30) ───────────────

/// Verifica que o resultado do comptime é o valor real (30), não o
/// SMI-tagged (61 = (30 << 1) | 1). Se o SMI decode estiver errado,
/// este teste falha com "61" em vez de "30".
#[test]
fn comptime_smi_not_tagged() {
    let (stdout, stderr, code) = run_kata_eval("@comptime + 10 20");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "30",
        "+ 10 20 deve produzir 30, não 61 (SMI) — stdout: {stdout}"
    );
}

// ── @comptime com subtração ────────────────────────────────────────

#[test]
fn comptime_sub_two_ints() {
    let (stdout, stderr, code) = run_kata_eval("@comptime - 10 3");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "7", "- 10 3 deve produzir 7 — stdout: {stdout}");
}

// ── @comptime com multiplicação ───────────────────────────────────

#[test]
fn comptime_mul_two_ints() {
    let (stdout, stderr, code) = run_kata_eval("@comptime * 6 7");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "42", "* 6 7 deve produzir 42 — stdout: {stdout}");
}

// ── @comptime sem efeito (sem @comptime produz o mesmo resultado) ──

/// Verifica que `+ 1 2` sem `@comptime` também produz 3 (sanity check
/// — o comptime pass não deve alterar o resultado).
#[test]
fn comptime_same_as_runtime() {
    let (with_comptime, _, _) = run_kata_eval("@comptime + 1 2");
    let (without_comptime, _, _) = run_kata_eval("+ 1 2");
    let a = with_comptime.lines().next().unwrap_or("");
    let b = without_comptime.lines().next().unwrap_or("");
    assert_eq!(a, b, "comptime e runtime devem produzir o mesmo valor");
}

// ── Fase 2: HeapSnapshot para tipos complexos ───────────────────

/// `@comptime [1 2 3]` deve serializar a lista como HeapSnapshot,
/// carregar na root_arena em load-time, e `len` deve retornar 3.
///
/// Este é o DoD da Fase 2: o ponteiro retornado por `kata_rt_get_snapshot`
/// é um Cons cell válido, navegável por `len`, `head`, `tail`.
#[test]
fn comptime_list_len_via_snapshot() {
    let (stdout, stderr, code) = run_kata_run("@comptime let x := [1 2 3]\nlen x");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "3",
        "len de @comptime [1 2 3] deve ser 3 — stdout: {stdout}"
    );
}

/// `head` sobre `@comptime [1 2 3]` deve retornar 1 (primeiro elemento).
#[test]
fn comptime_list_head_via_snapshot() {
    let (stdout, stderr, code) = run_kata_run("@comptime let x := [1 2 3]\nhead x");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "1",
        "head de @comptime [1 2 3] deve ser 1 — stdout: {stdout}"
    );
}

/// `head (tail x)` sobre `@comptime [1 2 3]` deve retornar 2.
#[test]
fn comptime_list_head_tail_via_snapshot() {
    let (stdout, stderr, code) = run_kata_run("@comptime let x := [1 2 3]\nhead (tail x)");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "2",
        "head (tail x) de @comptime [1 2 3] deve ser 2 — stdout: {stdout}"
    );
}

/// `len (tail (tail x))` sobre `@comptime [1 2 3]` deve retornar 1.
/// Exercita dois `tail` consecutivos — cada um desreferencia um
/// ponteiro no snapshot carregado.
#[test]
fn comptime_list_double_tail_len_via_snapshot() {
    let (stdout, stderr, code) = run_kata_run("@comptime let x := [1 2 3]\nlen (tail (tail x))");
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
    let (stdout, stderr, code) = run_kata_eval("@comptime [1 2 3]");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    // Ponteiro cru — um número grande (endereço na root_arena).
    assert!(
        first.parse::<i64>().is_ok(),
        "@comptime [1 2 3] deve retornar um ponteiro (i64), não crashar — stdout: {stdout}"
    );
}

// ── Fase 2: Text, Struct, Tuple, Sum via snapshot ────────────────

/// `@comptime "hello"` top-level deve imprimir `hello` — a string
/// é serializada para a appended section e o codegen faz `load(ptr+0)`
/// para obter o ponteiro da C string.
#[test]
fn comptime_text_top_level() {
    let (stdout, stderr, code) = run_kata_eval("@comptime \"hello\"");
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
    let (stdout, stderr, code) = run_kata_run("@comptime let x := \"hello\"\nlen x");
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
    let (stdout, stderr, code) = run_kata_run("@comptime let x := [\"a\" \"b\" \"c\"]\nlen x");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "3",
        "len de @comptime [\"a\" \"b\" \"c\"] deve ser 3 — stdout: {stdout}"
    );
}

/// `len (head x)` onde `x := @comptime ["hello" "world"]` → 5.
/// Exercita navegação de List de Text: head é um ponteiro para a
/// appended, `len` desreferencia e conta a string.
#[test]
fn comptime_list_of_text_head_len() {
    let (stdout, stderr, code) =
        run_kata_run("@comptime let x := [\"hello\" \"world\"]\nlen (head x)");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "5",
        "len (head x) de [\"hello\" \"world\"] deve ser 5 — stdout: {stdout}"
    );
}

/// `@comptime let p := Pessoa "Alice" 30` + `p.idade` → 30.
/// Struct com campo Text serializada via snapshot, acesso por campo.
#[test]
fn comptime_struct_field_access() {
    let src =
        "data Pessoa (nome::Text idade::Int)\n@comptime let p := Pessoa \"Alice\" 30\np.idade";
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
    let src =
        "data Pessoa (nome::Text idade::Int)\n@comptime let p := Pessoa \"Alice\" 30\nlen p.nome";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "5",
        "len p.nome de @comptime Pessoa deve ser 5 — stdout: {stdout}"
    );
}

/// `@comptime (1, 2, 3)` + `x.0` → 1. Tuple sem regressão.
#[test]
fn comptime_tuple_index_access() {
    let (stdout, stderr, code) = run_kata_run("@comptime let x := (1, 2, 3)\nx.0");
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "1",
        "x.0 de @comptime (1, 2, 3) deve ser 1 — stdout: {stdout}"
    );
}

/// `@comptime Result::Ok 42` + match → 42. Sum com payload Int
/// (SMI) serializado via snapshot, desempacotado por match.
#[test]
fn comptime_sum_int_match() {
    let src =
        "@comptime let r := Result::Ok 42\nmatch r\n    Result::Ok v: v\n    Result::Err e: 0";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "42",
        "match @comptime Result::Ok 42 deve produzir 42 — stdout: {stdout}"
    );
}

/// `@comptime Result::Err "fail"` + match → `fail`. Sum com payload
/// Text. O payload é copiado como ponteiro cru no serializer atual;
/// funciona porque a arena comptime sobrevive até o fim do processo.
#[test]
fn comptime_sum_text_match() {
    let src = "@comptime let r := Result::Err \"fail\"\nmatch r\n    Result::Ok v: v\n    Result::Err e: e";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "fail",
        "match @comptime Result::Err \"fail\" deve imprimir fail — stdout: {stdout}"
    );
}

/// `len e` no match de `@comptime Result::Err "fail"` → 4.
/// Exercita Text como payload de Sum acessado por `len`.
#[test]
fn comptime_sum_text_match_len() {
    let src = "@comptime let r := Result::Err \"fail\"\nmatch r\n    Result::Ok v: v\n    Result::Err e: len e";
    let (stdout, stderr, code) = run_kata_run(src);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "4",
        "len e de @comptime Result::Err \"fail\" deve ser 4 — stdout: {stdout}"
    );
}
