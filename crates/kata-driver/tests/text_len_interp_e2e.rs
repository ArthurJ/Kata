//! E2E — `len` de Text no interpretador (A3f: double SMI tag).
//!
//! Responsabilidade: cravar que `len` sobre Text retorna a contagem
//! correta de codepoints em ambos os backends. O interp aplicava
//! `encode_smi` sobre o retorno de `kata_rt_text_len` (que já é
//! SMI-tagged), causando double tagging — `len "abc"` retornava 7
//! em vez de 3.

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
        "kata_text_len_interp_e2e_{name}_{id}_{}.kata",
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

/// `len "abc"` retorna 3 — caso canônico do bug A3f (interp retornava 7).
#[test]
fn len_text_abc_retorna_3() {
    let source = r#"action main => Unit
    echo!(len "abc")
main!()"#;
    assert_both(source, "3\n");
}

/// `len ""` retorna 0 — string vazia.
#[test]
fn len_text_vazio_retorna_0() {
    let source = r#"action main => Unit
    echo!(len "")
main!()"#;
    assert_both(source, "0\n");
}

/// `len "café"` retorna 4 codepoints (não 5 bytes).
#[test]
fn len_text_com_acento_retorna_codepoints() {
    let source = r#"action main => Unit
    echo!(len "café")
main!()"#;
    assert_both(source, "4\n");
}

/// `len "日本語"` retorna 3 codepoints (CJK).
#[test]
fn len_text_cjk_retorna_codepoints() {
    let source = r#"action main => Unit
    echo!(len "日本語")
main!()"#;
    assert_both(source, "3\n");
}

/// `len` sobre Text em variável — não só literal.
#[test]
fn len_text_em_variavel() {
    let source = r#"action main => Unit
    let s := "Hello World"
    echo!(len s)
main!()"#;
    assert_both(source, "11\n");
}