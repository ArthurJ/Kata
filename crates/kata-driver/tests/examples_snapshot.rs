//! Snapshot tests dos exemplos `.kata` — roda `kata run` e compara stdout.
//!
//! Cada exemplo em `examples/*.kata` é executado via subprocesso `kata run`.
//! O stdout é capturado e comparado com um snapshot insta.
//!
//! Para aceitar mudanças: `cargo insta accept` (ou `INSTA_UPDATE=always cargo test`).

use std::fs;
use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata run <file>` e retorna stdout.
fn run_kata(file: &str) -> String {
    let output = Command::new(kata_bin())
        .args(["run", file])
        .output()
        .expect("executar kata run");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    // Se houve erro (stderr não vazio), incluir para diagnóstico.
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !stderr.is_empty()
        && !stderr.starts_with("    Finished")
        && !stderr.starts_with("     Running")
    {
        format!("{stdout}--- stderr ---\n{stderr}")
    } else {
        stdout
    }
}

/// Lista todos os arquivos `.kata` no diretório examples/.
fn example_files() -> Vec<String> {
    // CARGO_MANIFEST_DIR aponta para crates/kata-driver/ durante os testes.
    let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files: Vec<String> = Vec::new();
    let entries =
        fs::read_dir(&examples_dir).unwrap_or_else(|e| panic!("ler {:?}: {e}", examples_dir));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "kata") {
            files.push(path.to_string_lossy().to_string());
        }
    }
    files.sort();
    files
}

#[test]
fn snapshot_exemplos_kata() {
    let files = example_files();
    assert!(
        !files.is_empty(),
        "deve encontrar pelo menos 1 exemplo .kata"
    );

    for file in &files {
        let name = std::path::Path::new(file)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let output = run_kata(file);
        insta::assert_snapshot!(name, output);
    }
}
