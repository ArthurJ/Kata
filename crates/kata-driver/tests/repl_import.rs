//! Testes E2E do `kata repl` — import de módulos.

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
fn repl_import_selective() {
    // import examples/modules/mock_math.(dobrar) traz `dobrar` para o escopo.
    let out = run_repl(&[
        "import examples/modules/mock_math.(dobrar)",
        "echo!(dobrar 5)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "10"),
        "esperava 10 (dobrar 5), got: {out}"
    );
}

#[test]
fn repl_import_selective_alias() {
    // import com alias: dobrar as d → d 7 = 14
    let out = run_repl(&[
        "import examples/modules/mock_math.(dobrar as d)",
        "echo!(d 7)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "14"),
        "esperava 14 (d 7), got: {out}"
    );
}

#[test]
fn repl_import_persistent_across_lines() {
    // Import persiste entre linhas: dobrar disponível em múltiplas expressões.
    let out = run_repl(&[
        "import examples/modules/mock_math.(dobrar)",
        "echo!(dobrar 10)",
        "echo!(dobrar 20)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "20"),
        "esperava 20 (dobrar 10), got: {out}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "40"),
        "esperava 40 (dobrar 20), got: {out}"
    );
}

#[test]
fn repl_import_used_in_function() {
    // Função definida no REPL usa função importada.
    let out = run_repl(&[
        "import examples/modules/mock_math.(dobrar)",
        "quadrado :: Int => Int",
        "lambda x: * (dobrar x) (dobrar x)",
        "",
        "echo!(quadrado 5)",
        ":quit",
    ]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "100"),
        "esperava 100 (quadrado 5 = (dobrar 5) * (dobrar 5) = 10 * 10), got: {out}"
    );
}

#[test]
fn repl_load_with_import() {
    // :load de arquivo que contém import.
    // Cria arquivo temporário com import + função que usa o import.
    let temp = std::env::temp_dir().join("kata5_repl_load_import_test.kata");
    std::fs::write(&temp, "import examples/modules/mock_math.(dobrar)\n\ndobrar_e_somar :: Int Int => Int\nlambda a b: + (dobrar a) (dobrar b)\n\necho!(dobrar_e_somar 3 4)\n").unwrap();
    let path = temp.to_string_lossy().to_string();
    let out = run_repl(&[&format!(":load {path}"), ":quit"]);
    let lines = result_lines(&out);
    assert!(
        lines.iter().any(|l| l.trim() == "14"),
        "esperava 14 (dobrar 3 + dobrar 4 = 6 + 8), got: {out}"
    );
    let _ = std::fs::remove_file(&temp);
}
