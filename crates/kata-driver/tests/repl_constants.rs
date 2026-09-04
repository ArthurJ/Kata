//! Testes E2E do `kata repl` — `constant` em actions e funções (Fase 6).

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
fn repl_constant_in_action() {
    // constant scale := 2 → action foo (x::Int) => Int / * x scale → foo!(5) → 10
    // `foo 5` (sem `!`) é rejeitado pelo inference: action chamada sem `!`
    // é erro semântico, não bug de codegen. Usa `foo!(5)` para chamar actions.
    let out = run_repl(&[
        "constant scale := 2",
        "action foo (x::Int) => Int",
        "    * x scale",
        "",
        "foo!(5)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "10"),
        "esperava 10 (constant em action com foo!), got: {out}"
    );
    // Verifica que constant_fold substituiu scale (não há unbound ident).
    assert!(
        !out.contains("unbound ident: scale"),
        "constant_fold deveria substituir scale, got: {out}"
    );
}

#[test]
fn repl_constant_in_named_function() {
    // constant scale := 2 → double :: Int => Int / lambda x: * x scale → echo!(double 5) → 10
    let out = run_repl(&[
        "constant scale := 2",
        "double :: Int => Int",
        "lambda x: * x scale",
        "",
        "echo!(double 5)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "10"),
        "esperava 10 (constant em named function), got: {out}"
    );
}

#[test]
fn repl_constant_multiple_in_function() {
    // constant a := 3 → constant b := 4 → func :: Int => Int / lambda x: + (* x a) b
    // → echo!(func 2) → 10 (2*3 + 4)
    let out = run_repl(&[
        "constant a := 3",
        "constant b := 4",
        "func :: Int => Int",
        "lambda x: + (* x a) b",
        "",
        "echo!(func 2)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "10"),
        "esperava 10 (múltiplas constants em function), got: {out}"
    );
}

#[test]
fn repl_constant_shadowing() {
    // constant x := 10 → echo!(x) → 10 → constant x := 20 → echo!(x) → 20
    // No REPL, redefinir uma constant substitui a anterior. Em arquivo,
    // duplicatas são rejeitadas pela inference (DuplicateConstant).
    let out = run_repl(&[
        "constant x := 10",
        "echo!(x)",
        "constant x := 20",
        "echo!(x)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "10"),
        "esperava 10 (primeira constant), got: {out}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "20"),
        "esperava 20 (redefinição de constant), got: {out}"
    );
}

#[test]
fn repl_constant_and_let_in_function() {
    // constant scale := 2 → let n := 5 → func :: Int => Int / lambda x: + (* x scale) n
    // → echo!(func 10) → 25 (10*2 + 5)
    // NOTE: `let` no prompt NÃO é visível em functions (let é pre_entry,
    // function tem FunctionBuilder separado). Apenas `constant` é visível
    // via constant_fold. Para que `n` seja visível na function, deve ser
    // `constant n := 5`.
    let out = run_repl(&[
        "constant scale := 2",
        "constant n := 5",
        "func :: Int => Int",
        "lambda x: + (* x scale) n",
        "",
        "echo!(func 10)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "25"),
        "esperava 25 (constants em function), got: {out}"
    );
}

#[test]
fn repl_constant_float_in_function() {
    // constant pi := 3.14 → circle :: Float => Float / lambda r: * pi * r r
    // → echo!(circle 2.0) → 12.56
    let out = run_repl(&[
        "constant pi := 3.14",
        "circle :: Float => Float",
        "lambda r: * pi * r r",
        "",
        "echo!(circle 2.0)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    // 3.14 * 2.0 * 2.0 = 12.56
    assert!(
        lines.iter().any(|l| l
            .trim()
            .parse::<f64>()
            .is_ok_and(|v| (v - 12.56).abs() < 0.01)),
        "esperava ~12.56 (constant float em function), got: {out}"
    );
}
