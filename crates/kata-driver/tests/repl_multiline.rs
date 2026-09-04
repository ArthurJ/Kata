//! Testes E2E do `kata repl` — entrada multiline (match, enum, interface, sig).

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

// ── Multiline: match ───────────────────────────────────────

#[test]
fn repl_multiline_match_boolean() {
    let out = run_repl(&[
        "match = 1 1",
        "    True: \"igual\"",
        "    False: \"diferente\"",
        "",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("igual")),
        "esperava 'igual' em multiline match, got: {out}"
    );
}

#[test]
fn repl_multiline_match_then_expr() {
    // Match seguido de expressão — ambos devem funcionar.
    let out = run_repl(&[
        "match = 1 1",
        "    True: \"igual\"",
        "    False: \"diferente\"",
        "",
        "+ 1 2",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("igual")),
        "esperava 'igual', got: {out}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "3"),
        "esperava 3 após match, got: {out}"
    );
}

#[test]
fn repl_multiline_match_user_enum() {
    // Declara enum, depois match nele.
    let out = run_repl(&[
        "enum Cor",
        "    Vermelho",
        "    Verde",
        "    Azul",
        "",
        "match Cor::Verde",
        "    Cor::Vermelho: \"red\"",
        "    Cor::Verde: \"green\"",
        "    Cor::Azul: \"blue\"",
        "",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("green")),
        "esperava 'green' em match enum, got: {out}"
    );
}

// ── Multiline: enum ────────────────────────────────────────

#[test]
fn repl_multiline_enum_decl() {
    let out = run_repl(&[
        "enum Cor",
        "    Vermelho",
        "    Verde",
        "    Azul",
        "",
        ":env",
        ":quit",
    ]);
    let lines = result_lines(&out);
    // :env deve mostrar o binding se houver, mas pelo menos
    // não deve haver erro de parse.
    assert!(
        !lines.iter().any(|l| l.contains("erro de parse")),
        "não esperava erro de parse em enum, got: {out}"
    );
}

#[test]
fn repl_multiline_enum_then_use() {
    let out = run_repl(&[
        "enum Cor",
        "    Vermelho",
        "    Verde",
        "    Azul",
        "",
        "constant c := Cor::Verde",
        ":type c",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.contains("Cor")),
        "esperava tipo Cor em :type c, got: {out}"
    );
}

// ── Multiline: interface ───────────────────────────────────

#[test]
fn repl_multiline_interface_two_methods() {
    let out = run_repl(&[
        "interface CUSTOM_IFACE",
        "    foo :: Int => Int",
        "    bar :: Int => Int",
        "",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        !lines.iter().any(|l| l.contains("erro de parse")),
        "não esperava erro de parse em interface, got: {out}"
    );
}

#[test]
fn repl_multiline_implements() {
    // implements também é um bloco indentado.
    let out = run_repl(&[
        "Int implements SHOW",
        "    @ffi(\"kata_rt_bi_show\")",
        "    show :: Int => Text",
        "",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        !lines.iter().any(|l| l.contains("erro de parse")),
        "não esperava erro de parse em implements, got: {out}"
    );
}

// ── Multiline: sig ─────────────────────────────────────────

#[test]
fn repl_multiline_sig_with_clauses() {
    // Envia Sig + 2 cláusulas no mesmo nível + linha em branco + chamada.
    let out = run_repl(&[
        "fat :: Int Int => Int",
        "lambda 0 acc: acc",
        "lambda n acc: fat (- n 1) (* n acc)",
        "",
        "fat 5 1",
        ":quit",
    ]);
    let lines = result_lines(&out);
    // 5! = 120
    assert!(
        lines.iter().any(|l| l.trim() == "120"),
        "esperava 120 em multiline, got: {out}"
    );
}

#[test]
fn repl_multiline_sig_decl_only() {
    // Sig + cláusulas sem entry — apenas declara, não executa.
    let out = run_repl(&[
        "fat :: Int Int => Int",
        "lambda 0 acc: acc",
        "lambda n acc: fat (- n 1) (* n acc)",
        "",
        "fat 3 1",
        ":quit",
    ]);
    let lines = result_lines(&out);
    // 3! = 6
    assert!(
        lines.iter().any(|l| l.trim() == "6"),
        "esperava 6 em multiline decl, got: {out}"
    );
}

#[test]
fn repl_single_line_still_works() {
    // Expressão de uma linha não deve ativar multiline.
    let out = run_repl(&["+ 1 2", ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "3"),
        "esperava 3, got: {out}"
    );
}
