//! Testes E2E do `kata repl` — comandos `:type`, `:env`, `:reset`, `:help`.

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
fn repl_type_command() {
    let out = run_repl(&["constant x := 10", ":type x", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("Int")),
        "esperava Int, got: {out}"
    );
}

#[test]
fn repl_type_expr_arith() {
    let out = run_repl(&[":type + 1 2", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("Int")),
        "esperava Int, got: {out}"
    );
}

#[test]
fn repl_env_shows_bindings_with_types() {
    let out = run_repl(&["let x := 10", ":env", ":quit"]);
    let lines = result_lines(&out);
    // :env deve listar 'x: Int'
    assert!(
        lines.iter().any(|l| l.contains('x') && l.contains("Int")),
        "esperava x: Int no :env, got: {out}"
    );
}

#[test]
fn repl_env_empty() {
    let out = run_repl(&[":env", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("nenhum")),
        "esperava nenhum binding, got: {out}"
    );
}

#[test]
fn repl_reset_clears_bindings() {
    let out = run_repl(&["constant x := 10", ":reset", ":env", ":quit"]);
    let lines = result_lines(&out);
    // Após reset, :env deve mostrar nenhum binding.
    assert!(
        lines.iter().any(|l| l.contains("nenhum")),
        "esperava nenhum binding após :reset, got: {out}"
    );
}

#[test]
fn repl_reset_reloads_prelude() {
    let out = run_repl(&["constant x := 10", ":reset", "+ 1 2", ":quit"]);
    let lines = result_lines(&out);
    // Após reset, + 1 2 deve funcionar (prelude recarregado).
    assert!(
        lines.iter().any(|l| l.trim() == "3"),
        "esperava 3 após reset, got: {out}"
    );
}

#[test]
fn repl_help_lists_commands() {
    let out = run_repl(&[":help", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains(":type")),
        "esperava :type no help, got: {out}"
    );
    assert!(
        lines.iter().any(|l| l.contains(":env")),
        "esperava :env no help, got: {out}"
    );
    assert!(
        lines.iter().any(|l| l.contains(":quit")),
        "esperava :quit no help, got: {out}"
    );
}

#[test]
fn repl_unknown_command_reports_error() {
    let out = run_repl(&[":bogus", ":quit"]);
    assert!(
        out.contains("desconhecido"),
        "esperava mensagem de comando desconhecido, got: {out}"
    );
}
