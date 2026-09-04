//! E2E — `contains` em Range via chamada direta e operador `in`.
//!
//! Responsabilidade: cravar que `contains [1..10] 5` e `5 in [1..10]`
//! retornam o resultado correto em ambos os backends. Antes do fix,
//! `contains [1..10] 5` produzia `codegen.ffi_not_found: range_contains`
//! no JIT e `FFI não implementado no interpretador: range_contains` no
//! interp — o inline só existia no braço `In` do `lower_expr` (operador
//! `in`), não no `lower_closure` (chamada direta via `contains`).

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
        "kata_range_contains_e2e_{name}_{id}_{}.kata",
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

// ── Chamada direta: contains ───────────────────────────────────

/// `contains [1..10] 5` → True. Caso canônico do bug.
#[test]
fn contains_range_exclusivo_dentro() {
    let source = r#"action main => Unit
    echo!(contains [1..10] 5)
main!()"#;
    assert_both(source, "True\n");
}

/// `contains [1..10] 0` → False (fora do intervalo).
#[test]
fn contains_range_exclusivo_fora() {
    let source = r#"action main => Unit
    echo!(contains [1..10] 0)
main!()"#;
    assert_both(source, "False\n");
}

/// `contains [1..10] 10` → False (exclusive não inclui end).
#[test]
fn contains_range_exclusivo_end() {
    let source = r#"action main => Unit
    echo!(contains [1..10] 10)
main!()"#;
    assert_both(source, "False\n");
}

/// `contains [1..=10] 10` → True (inclusive inclui end).
#[test]
fn contains_range_inclusivo_end() {
    let source = r#"action main => Unit
    echo!(contains [1..=10] 10)
main!()"#;
    assert_both(source, "True\n");
}

/// `contains [1..=10] 11` → False (além do end inclusivo).
#[test]
fn contains_range_inclusivo_fora() {
    let source = r#"action main => Unit
    echo!(contains [1..=10] 11)
main!()"#;
    assert_both(source, "False\n");
}

/// `contains [0..2..10] 4` → True (alinhado com step 2).
#[test]
fn contains_range_step_alinhado() {
    let source = r#"action main => Unit
    echo!(contains [0..2..10] 4)
main!()"#;
    assert_both(source, "True\n");
}

/// `contains [0..2..10] 5` → False (não alinhado com step 2).
#[test]
fn contains_range_step_nao_alinhado() {
    let source = r#"action main => Unit
    echo!(contains [0..2..10] 5)
main!()"#;
    assert_both(source, "False\n");
}

/// `contains [0..2..=10] 10` → True (inclusive, alinhado).
#[test]
fn contains_range_step_inclusivo_end() {
    let source = r#"action main => Unit
    echo!(contains [0..2..=10] 10)
main!()"#;
    assert_both(source, "True\n");
}

/// `contains [10..-1..1] 5` → True (decrescente).
#[test]
fn contains_range_decrescente_dentro() {
    let source = r#"action main => Unit
    echo!(contains [10..-1..1] 5)
main!()"#;
    assert_both(source, "True\n");
}

/// `contains [10..-1..1] 0` → False (fora do intervalo decrescente).
#[test]
fn contains_range_decrescente_fora() {
    let source = r#"action main => Unit
    echo!(contains [10..-1..1] 0)
main!()"#;
    assert_both(source, "False\n");
}

/// `contains [10..-2..1] 8` → True (decrescente, step 2, alinhado).
#[test]
fn contains_range_decrescente_step() {
    let source = r#"action main => Unit
    echo!(contains [10..-2..1] 8)
main!()"#;
    assert_both(source, "True\n");
}

/// `contains [5..1] 3` → False (range vazio, step positivo, start > end).
#[test]
fn contains_range_vazio() {
    let source = r#"action main => Unit
    echo!(contains [5..1] 3)
main!()"#;
    assert_both(source, "False\n");
}

// ── Operador in (refatoração não deve ter quebrado) ────────────

/// `5 in [1..10]` → True.
#[test]
fn in_op_range_dentro() {
    let source = r#"action main => Unit
    echo!(5 in [1..10])
main!()"#;
    assert_both(source, "True\n");
}

/// `0 in [1..10]` → False.
#[test]
fn in_op_range_fora() {
    let source = r#"action main => Unit
    echo!(0 in [1..10])
main!()"#;
    assert_both(source, "False\n");
}

/// `10 in [1..=10]` → True (inclusive).
#[test]
fn in_op_range_inclusivo() {
    let source = r#"action main => Unit
    echo!(10 in [1..=10])
main!()"#;
    assert_both(source, "True\n");
}

/// `4 in [0..2..10]` → True (step alinhado).
#[test]
fn in_op_range_step_alinhado() {
    let source = r#"action main => Unit
    echo!(4 in [0..2..10])
main!()"#;
    assert_both(source, "True\n");
}

/// `5 in [0..2..10]` → False (step não alinhado).
#[test]
fn in_op_range_step_nao_alinhado() {
    let source = r#"action main => Unit
    echo!(5 in [0..2..10])
main!()"#;
    assert_both(source, "False\n");
}

/// `5 in [10..-1..1]` → True (decrescente).
#[test]
fn in_op_range_decrescente() {
    let source = r#"action main => Unit
    echo!(5 in [10..-1..1])
main!()"#;
    assert_both(source, "True\n");
}

// ── Em variável ─────────────────────────────────────────────────

/// `contains` com Range em variável.
#[test]
fn contains_range_em_variavel() {
    let source = r#"action main => Unit
    let r := [1..10]
    echo!(contains r 5)
main!()"#;
    assert_both(source, "True\n");
}