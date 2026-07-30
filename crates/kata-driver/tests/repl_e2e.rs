//! Testes E2E do `kata repl` — sessão REPL interativa.
//!
//! Cada teste envia uma sequência de inputs via stdin (pipe) para o
//! binário `kata repl`, verifica o stdout produzido, e confirma o
//! comportamento esperado: avaliação de expressões, persistência de
//! `let` bindings entre iterações, comandos `:type`/`:env`/`:reset`,
//! e rollback em caso de erro.
//!
//! Os testes assumem que o binário `kata` foi compilado pelo cargo
//! antes dos testes rodarem (padrão `cargo test --workspace` já
//! compila o binário do driver).

use std::io::Write;
use std::process::{Command, Stdio};

/// Retorna o path do binário `kata` compilado pelo cargo.
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata repl` com stdin pipe, envia os inputs sequenciais
/// (um por linha), e retorna o stdout completo.
fn run_repl(inputs: &[&str]) -> String {
    let mut child = Command::new(kata_bin())
        .arg("repl")
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
        .filter(|l| !l.starts_with("Kata REPL") && !l.starts_with("kata>"))
        .collect()
}

// ── Avaliação básica ────────────────────────────────────────

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

// ── Persistência de bindings ───────────────────────────────

#[test]
fn repl_let_binding_persists() {
    let out = run_repl(&["let x := 10", "+ x 5", ":quit"]);
    let lines = result_lines(&out);
    // + x 5 deve produzir 15
    assert!(
        lines.iter().any(|l| l.trim() == "15"),
        "esperava 15 após persistência, got: {out}"
    );
}

#[test]
fn repl_multiple_let_bindings() {
    let out = run_repl(&["let x := 10", "let y := 20", "+ x y", ":quit"]);
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

// ── Comandos :type, :env, :reset ────────────────────────────

#[test]
fn repl_type_command() {
    let out = run_repl(&["let x := 10", ":type x", ":quit"]);
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
    let out = run_repl(&["let x := 10", ":reset", ":env", ":quit"]);
    let lines = result_lines(&out);
    // Após reset, :env deve mostrar nenhum binding.
    assert!(
        lines.iter().any(|l| l.contains("nenhum")),
        "esperava nenhum binding após :reset, got: {out}"
    );
}

#[test]
fn repl_reset_reloads_prelude() {
    let out = run_repl(&["let x := 10", ":reset", "+ 1 2", ":quit"]);
    let lines = result_lines(&out);
    // Após reset, + 1 2 deve funcionar (prelude recarregado).
    assert!(
        lines.iter().any(|l| l.trim() == "3"),
        "esperava 3 após reset, got: {out}"
    );
}

// ── Help e quit ────────────────────────────────────────────

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

// ── Erros não abortam sessão ────────────────────────────────

#[test]
fn repl_error_does_not_abort_session() {
    let out = run_repl(&["undefined_name", "+ 1 2", ":quit"]);
    let lines = result_lines(&out);
    // Após erro, + 1 2 deve funcionar.
    assert!(
        lines.iter().any(|l| l.trim() == "3"),
        "esperava 3 após erro, got: {out}"
    );
}

#[test]
fn repl_rollback_on_error_keeps_env() {
    let out = run_repl(&["let x := 10", "undefined_name", ":env", ":quit"]);
    let lines = result_lines(&out);
    // x deve continuar no :env após erro.
    assert!(
        lines.iter().any(|l| l.contains('x') && l.contains("Int")),
        "esperava x: Int no :env após erro, got: {out}"
    );
}

// ── :load ──────────────────────────────────────────────────

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
    let path = write_temp_kata("load_let", "let x := 42\n");
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
        "let c := Cor::Verde",
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
        "    show :: Int => Text @ffi(\"kata_rt_bi_show\")",
        "",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        !lines.iter().any(|l| l.contains("erro de parse")),
        "não esperava erro de parse em implements, got: {out}"
    );
}

// ── Multiline ──────────────────────────────────────────────

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
