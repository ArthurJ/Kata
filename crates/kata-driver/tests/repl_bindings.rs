//! Testes E2E do `kata repl` — persistência de `let` bindings.

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
fn repl_let_binding_persists() {
    let out = run_repl(&["constant x := 10", "+ x 5", ":quit"]);
    let lines = result_lines(&out);
    // + x 5 deve produzir 15
    assert!(
        lines.iter().any(|l| l.trim() == "15"),
        "esperava 15 após persistência, got: {out}"
    );
}

#[test]
fn repl_multiple_let_bindings() {
    let out = run_repl(&["constant x := 10", "constant y := 20", "+ x y", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "30"),
        "esperava 30, got: {out}"
    );
}

#[test]
fn repl_let_shadowing() {
    let out = run_repl(&["let x := 10", "let x := 20", "x", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "20"),
        "esperava 20 (shadowing), got: {out}"
    );
}
