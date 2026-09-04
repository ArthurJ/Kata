//! Testes E2E do `kata repl` — avaliação básica de expressões.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Retorna o path absoluto do binário `kata` compilado pelo cargo.
fn kata_bin() -> String {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(".").to_path_buf());
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| {
            workspace
                .join("target/debug/kata")
                .to_string_lossy()
                .to_string()
        })
}

/// Executa `kata repl` com stdin pipe, envia os inputs sequenciais
/// (um por linha), e retorna o stdout completo.
fn run_repl(inputs: &[&str]) -> String {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(".").to_path_buf());

    let mut child = Command::new(kata_bin())
        .arg("repl")
        .current_dir(&workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("executar kata repl");

    {
        let stdin = child.stdin.as_mut().expect("stdin pipe");
        for line in inputs {
            writeln!(stdin, "{line}").expect("escrever stdin");
        }
    }

    let output = child.wait_with_output().expect("esperar kata repl");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}{stderr}")
    }
}

/// Extrai as linhas de resultado (não prompt, não banner) do output.
fn result_lines(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter(|l| !l.starts_with("Kata REPL") && !l.starts_with(">>>"))
        .collect()
}

#[test]
fn repl_eval_int_arith() {
    let out = run_repl(&["+ 1 2", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains('3')),
        "esperava 3, got: {out}"
    );
}

#[test]
fn repl_eval_float() {
    let out = run_repl(&["+ 1.5 2.5", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains('4')),
        "esperava 4, got: {out}"
    );
}

#[test]
fn repl_eval_text_literal() {
    // Envia "hello" (literal Text) ao REPL.
    let out = run_repl(&["\"hello\"", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("hello")),
        "esperava hello, got: {out}"
    );
}

#[test]
fn repl_eval_boolean() {
    let out = run_repl(&["True", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("True")),
        "esperava True, got: {out}"
    );
}
