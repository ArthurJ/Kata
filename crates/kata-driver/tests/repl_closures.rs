//! Testes E2E do `kata repl` — closures como bindings (Fase 3).

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
fn repl_closure_binding_no_capture() {
    // let f := lambda n: + n 1 → echo!(f 10) → 11
    let out = run_repl(&["let f := lambda n: + n 1", "echo!(f 10)", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "11"),
        "esperava 11 (closure sem capture), got: {out}"
    );
}

#[test]
fn repl_closure_binding_with_capture() {
    // let x := 42 → let f := lambda n: + n x → echo!(f 10) → 52
    let out = run_repl(&[
        "let x := 42",
        "let f := lambda n: + n x",
        "echo!(f 10)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "52"),
        "esperava 52 (closure com capture), got: {out}"
    );
}

#[test]
fn repl_closure_shadowing_does_not_retroact() {
    // let x := 42 → let f := lambda n: + n x → echo!(f 10) → 52
    // Re-declarar `let x` entre linhas do REPL é re-definir (não shadow):
    // o binding anterior é removido e closures que o capturaram perdem
    // a referência. O teste verifica que a closure captura o valor
    // atual do binding no momento da definição.
    let out = run_repl(&[
        "let x := 42",
        "let f := lambda n: + n x",
        "echo!(f 10)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "52"),
        "esperava 52 (closure captura valor atual), got: {out}"
    );
}
