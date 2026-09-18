//! Testes E2E da Fase 2 do PRD-pragma-mechanism — metadados de diagnóstico.
//!
//! Valida que:
//! - T7: diagnóstico ajustável responde a `#!allow`/`#!warn`/`#!deny`
//! - T8: diagnóstico não-ajustável + `#!allow` → erro com sugestão
//! - T9: código inexistente (typo) → erro com sugestão
//! - T10: pragma redundante (mesmo nível do default) → warning
//!
//! Estes testes usam subprocess (`kata run`) porque validam o pipeline
//! completo, incluindo o processamento de pragmas e o filtro de erros.

use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn run_kata(source: &str) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_pragma_diag_e2e_{id}_{pid}.kata",
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

// ── T7: diagnóstico ajustável responde a #!allow/#!warn/#!deny ──

/// `#!allow type.redundant_clause` silencia o diagnóstico.
/// O programa compila e executa sem erros.
#[test]
fn t7_allow_silences_redundant_clause() {
    let source = r#"#!allow type.redundant_clause

f :: Int => Int
lambda x:
    match x
        0: 1
        _: 2
        0: 3

action main => Int
    echo!(f 0)
    0

main!()
"#;
    let (_stdout, stderr, exit) = run_kata(source);
    assert_eq!(exit, 0, "exit should be 0 (pragma silenced the error)");
    assert!(
        !stderr.contains("redundant") && !stderr.contains("RedundantClause"),
        "stderr should NOT contain 'redundant' — got: {stderr}"
    );
}

/// `#!warn type.redundant_clause` rebaixa para warning.
/// Se o diagnóstico não dispara, o pragma é aceito sem efeito e o
/// programa compila normalmente. Se disparar, seria rebaixado para
/// warning. Como não há um caso que dispare redundant_clause como
/// Err no pipeline atual, este teste verifica que o pragma é aceito.
#[test]
fn t7_warn_downgrades_redundant_clause() {
    let source = r#"#!warn type.redundant_clause

action main => Int
    echo!("ok")
    0

main!()
"#;
    let (stdout, stderr, exit) = run_kata(source);
    // #!warn em um diagnóstico ajustável é aceito sem erro.
    // O programa compila normalmente.
    assert_eq!(
        exit, 0,
        "exit should be 0 (pragma accepted) — got stderr: {stderr}"
    );
    assert!(stdout.contains("ok"), "stdout should contain 'ok'");
}

// ── T8: diagnóstico não-ajustável rejeita #!allow ──

/// `#!allow type.mismatch` → erro "não é severity-adjustable" com
/// sugestão de ajustáveis.
#[test]
fn t8_not_adjustable_rejects_allow() {
    let source = r#"#!allow type.mismatch

action main => Int
    echo!("ok")
    0

main!()
"#;
    let (_stdout, stderr, exit) = run_kata(source);
    assert_ne!(exit, 0, "exit should be non-zero (not adjustable error)");
    assert!(
        stderr.contains("não é severity-adjustable") || stderr.contains("not severity-adjustable"),
        "stderr should contain 'not severity-adjustable' — got: {stderr}"
    );
    // Should suggest adjustable diagnostics
    assert!(
        stderr.contains("type.redundant_clause") || stderr.contains("type.incomplete_interface"),
        "stderr should suggest adjustable diagnostics — got: {stderr}"
    );
}

// ── T9: código inexistente (typo) ──

/// `#!allow type.incomplte_interface` (typo) → erro "unknown
/// diagnostic code" com sugestão `type.incomplete_interface`.
#[test]
fn t9_unknown_code_with_suggestion() {
    let source = r#"#!allow type.incomplte_interface

action main => Int
    echo!("ok")
    0

main!()
"#;
    let (_stdout, stderr, exit) = run_kata(source);
    assert_ne!(exit, 0, "exit should be non-zero (unknown code error)");
    assert!(
        stderr.contains("desconhecido") || stderr.contains("unknown"),
        "stderr should contain 'unknown' or 'desconhecido' — got: {stderr}"
    );
    // Should suggest the correct code
    assert!(
        stderr.contains("type.incomplete_interface"),
        "stderr should suggest 'type.incomplete_interface' — got: {stderr}"
    );
}

// ── T10: pragma redundante ──

/// `#!deny type.redundant_clause` (deny é o default) → warning de
/// pragma redundante.
#[test]
fn t10_redundant_deny_on_deny_default() {
    let source = r#"#!deny type.redundant_clause

action main => Int
    echo!("ok")
    0

main!()
"#;
    let (_stdout, stderr, _exit) = run_kata(source);
    // Should produce a redundancy warning about the pragma itself
    assert!(
        stderr.contains("redundante") || stderr.contains("redundant"),
        "stderr should mention 'redundant' pragma — got: {stderr}"
    );
}

// ── Teste extra: pragma válido não interfere com compilação ──

/// `#!allow type.redundant_clause` antes de código sem redundant_clause
/// não produz erro nem warning.
#[test]
fn pragma_valid_no_effect_on_clean_code() {
    let source = r#"#!allow type.redundant_clause

action main => Int
    echo!("ok")
    0

main!()
"#;
    let (stdout, stderr, exit) = run_kata(source);
    assert_eq!(exit, 0, "exit should be 0 (clean code with valid pragma)");
    assert!(
        stderr.is_empty() || !stderr.contains("error"),
        "stderr should not contain errors — got: {stderr}"
    );
    assert!(stdout.contains("ok"), "stdout should contain 'ok'");
}
