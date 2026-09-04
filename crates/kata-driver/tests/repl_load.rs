//! Testes E2E do `kata repl` — comando `:load`.

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

/// Cria um arquivo .kata temporário e retorna o path.
fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e-repl");
    std::fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    std::fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

#[test]
fn repl_load_fatorial() {
    let path = write_temp_kata(
        "load_fatorial",
        "fat :: Int Int => Int\nlambda 0 acc: acc\nlambda n acc: fat (- n 1) (* n acc)\n\nfat 5 1\n",
    );
    let out = run_repl(&[&format!(":load {path}"), ":quit"]);
    let lines = result_lines(&out);
    // fat 5 1 = 120
    assert!(
        lines.iter().any(|l| l.trim() == "120"),
        "esperava 120 após :load fatorial, got: {out}"
    );
}

#[test]
fn repl_load_makes_function_available() {
    let path = write_temp_kata(
        "load_func",
        "fat :: Int Int => Int\nlambda 0 acc: acc\nlambda n acc: fat (- n 1) (* n acc)\n",
    );
    // Carrega o arquivo (sem entry) e depois chama fat 6 1.
    let out = run_repl(&[&format!(":load {path}"), "fat 6 1", ":quit"]);
    let lines = result_lines(&out);
    // 6! = 720
    assert!(
        lines.iter().any(|l| l.trim() == "720"),
        "esperava 720 após :load + fat 6 1, got: {out}"
    );
}

#[test]
fn repl_load_let_binding_persists() {
    let path = write_temp_kata("load_let", "constant x := 42\n");
    let out = run_repl(&[&format!(":load {path}"), "+ x 1", ":quit"]);
    let lines = result_lines(&out);
    // x = 42, + x 1 = 43
    assert!(
        lines.iter().any(|l| l.trim() == "43"),
        "esperava 43 após :load let + + x 1, got: {out}"
    );
}

#[test]
fn repl_load_nonexistent_file_reports_error() {
    let out = run_repl(&[":load /tmp/nonexistent_kata_file.kata", ":quit"]);
    assert!(
        out.contains("não foi possível ler") || out.contains("erro"),
        "esperava mensagem de erro para arquivo inexistente, got: {out}"
    );
}
