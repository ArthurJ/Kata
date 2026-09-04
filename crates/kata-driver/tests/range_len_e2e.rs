//! E2E — `len` em Range (A3d: ffi_not_found → inline O(1)).
//!
//! Responsabilidade: cravar que `len` sobre Range retorna a contagem
//! correta de elementos em ambos os backends. Antes do fix, `len [1..10]`
//! produzia `codegen.ffi_not_found: range_len` no JIT e `FFI não
//! implementado no interpretador: range_len` no interp — não existia
//! FFI nem inline para `range_len`, apenas `@builtin("range_len")` no
//! `implements COUNTABLE` de Range em `core.kata`.

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
fn write_temp(name: &str, src: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kata_range_len_e2e_{name}_{id}_{}.kata",
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().to_string()
}

/// Ambos os backends devem ter exit 0 e o mesmo stdout.
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

// ── Exclusive (..) ──────────────────────────────────────────────

/// `[1..10]` → 9 elementos (1 a 9). Caso canônico do bug A3d.
#[test]
fn len_range_exclusivo_simples() {
    let source = r#"action main => Unit
    echo!(len [1..10])
main!()"#;
    assert_both(source, "9\n");
}

/// `[0..10]` → 10 elementos.
#[test]
fn len_range_exclusivo_de_zero() {
    let source = r#"action main => Unit
    echo!(len [0..10])
main!()"#;
    assert_both(source, "10\n");
}

/// `[5..1]` → 0 (vazio, start > end sem step negativo).
#[test]
fn len_range_exclusivo_vazio() {
    let source = r#"action main => Unit
    echo!(len [5..1])
main!()"#;
    assert_both(source, "0\n");
}

/// `[5..6]` → 1 elemento.
#[test]
fn len_range_exclusivo_unitario() {
    let source = r#"action main => Unit
    echo!(len [5..6])
main!()"#;
    assert_both(source, "1\n");
}

// ── Inclusive (..=) ─────────────────────────────────────────────

/// `[1..=10]` → 10 elementos (1 a 10).
#[test]
fn len_range_inclusivo_simples() {
    let source = r#"action main => Unit
    echo!(len [1..=10])
main!()"#;
    assert_both(source, "10\n");
}

/// `[5..=5]` → 1 elemento.
#[test]
fn len_range_inclusivo_unitario() {
    let source = r#"action main => Unit
    echo!(len [5..=5])
main!()"#;
    assert_both(source, "1\n");
}

// ── Com step (..s..) ────────────────────────────────────────────

/// `[0..2..10]` → 5 elementos (0, 2, 4, 6, 8).
#[test]
fn len_range_step_par_exclusivo() {
    let source = r#"action main => Unit
    echo!(len [0..2..10])
main!()"#;
    assert_both(source, "5\n");
}

/// `[0..3..10]` → 4 elementos (0, 3, 6, 9) — step não divide diff.
#[test]
fn len_range_step_nao_divide_exclusivo() {
    let source = r#"action main => Unit
    echo!(len [0..3..10])
main!()"#;
    assert_both(source, "4\n");
}

/// `[0..2..=10]` → 6 elementos (0, 2, 4, 6, 8, 10).
#[test]
fn len_range_step_par_inclusivo() {
    let source = r#"action main => Unit
    echo!(len [0..2..=10])
main!()"#;
    assert_both(source, "6\n");
}

// ── Decrescente ─────────────────────────────────────────────────

/// `[10..-1..1]` → 9 elementos (10 a 2).
#[test]
fn len_range_decrescente_exclusivo() {
    let source = r#"action main => Unit
    echo!(len [10..-1..1])
main!()"#;
    assert_both(source, "9\n");
}

/// `[10..-1..=1]` → 10 elementos (10 a 1).
#[test]
fn len_range_decrescente_inclusivo() {
    let source = r#"action main => Unit
    echo!(len [10..-1..=1])
main!()"#;
    assert_both(source, "10\n");
}

/// `[10..-2..1]` → 5 elementos (10, 8, 6, 4, 2).
#[test]
fn len_range_decrescente_step() {
    let source = r#"action main => Unit
    echo!(len [10..-2..1])
main!()"#;
    assert_both(source, "5\n");
}

// ── Em variável ─────────────────────────────────────────────────

/// `len` sobre Range em variável — não só literal.
#[test]
fn len_range_em_variavel() {
    let source = r#"action main => Unit
    let r := [1..10]
    echo!(len r)
main!()"#;
    assert_both(source, "9\n");
}

// ── Range grande (O(1), não materializa) ───────────────────────

/// `[0..1..1000000]` → 1000000 — confirma O(1) (não materializa lista).
#[test]
fn len_range_grande_o1() {
    let source = r#"action main => Unit
    echo!(len [0..1..1000000])
main!()"#;
    assert_both(source, "1000000\n");
}
