//! Testes E2E do `kata repl` — funções nomeadas (Fase 4).

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
fn repl_named_function_persists() {
    // double :: Int => Int / lambda x: * x 2 → echo!(double 5) → 10
    let out = run_repl(&[
        "double :: Int => Int",
        "lambda x: * x 2",
        "",
        "echo!(double 5)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "10"),
        "esperava 10 (named function persiste), got: {out}"
    );
}

#[test]
fn repl_named_function_nested_call() {
    // double :: Int => Int / lambda x: * x 2 → echo!(double (double 5)) → 20
    let out = run_repl(&[
        "double :: Int => Int",
        "lambda x: * x 2",
        "",
        "echo!(double (double 5))",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "20"),
        "esperava 20 (nested call), got: {out}"
    );
}

#[test]
fn repl_named_function_multiple_calls() {
    // Definir double, chamar 3 vezes em linhas separadas
    let out = run_repl(&[
        "double :: Int => Int",
        "lambda x: * x 2",
        "",
        "echo!(double 5)",
        "echo!(double 10)",
        "echo!(double (double 3))",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "10"),
        "esperava 10 (double 5), got: {out}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "20"),
        "esperava 20 (double 10), got: {out}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "12"),
        "esperava 12 (double (double 3)), got: {out}"
    );
}

#[test]
fn repl_named_function_recursive() {
    // fat :: Int Int => Int / lambda 0 acc: acc / lambda n acc: fat (- n 1) (* n acc)
    // echo!(fat 5 1) → 120, echo!(fat 10 1) → 3628800
    let out = run_repl(&[
        "fat :: Int Int => Int",
        "lambda 0 acc: acc",
        "lambda n acc: fat (- n 1) (* n acc)",
        "",
        "echo!(fat 5 1)",
        "echo!(fat 10 1)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "120"),
        "esperava 120 (fat 5 1), got: {out}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "3628800"),
        "esperava 3628800 (fat 10 1), got: {out}"
    );
}

#[test]
fn repl_named_function_redeclare() {
    // Redefinir double com corpo diferente — shadowing de função
    let out = run_repl(&[
        "double :: Int => Int",
        "lambda x: * x 2",
        "",
        "echo!(double 5)",
        "double :: Int => Int",
        "lambda x: + x 100",
        "",
        "echo!(double 5)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    // Primeira versão: * 5 2 = 10
    assert!(
        lines.iter().any(|l| l.trim() == "10"),
        "esperava 10 (primeira versão), got: {out}"
    );
    // Segunda versão: + 5 100 = 105
    assert!(
        lines.iter().any(|l| l.trim() == "105"),
        "esperava 105 (segunda versão), got: {out}"
    );
}
