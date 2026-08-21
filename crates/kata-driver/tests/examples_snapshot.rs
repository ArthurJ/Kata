//! Snapshot tests dos exemplos `.kata` — roda `kata run` e compara stdout.
//!
//! Cada exemplo em `examples/*.kata` (recursivo, incluindo subdiretórios)
//! é executado via subprocesso `kata run`. O stdout é capturado e comparado
//! com um snapshot insta.
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

/// Lista todos os arquivos `.kata` no diretório examples/ (recursivo).
///
/// Arquivos em subdiretórios (ex: `examples/modules/mock_math.kata`) são
/// incluídos. O nome do snapshot é o caminho relativo sem extensão
/// (ex: `modules/imports`).
///
/// Arquivos que não são entrypoints (sem `main!()` ou expressão top-level)
/// são pulados se produzirem erro de `<entry point>`.
fn example_files() -> Vec<(String, String)> {
    // CARGO_MANIFEST_DIR aponta para crates/kata-driver/ durante os testes.
    let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files: Vec<(String, String)> = Vec::new();
    collect_kata_recursive(&examples_dir, &examples_dir, &mut files);
    files.sort();
    files
}

/// Coleta arquivos `.kata` recursivamente.
///
/// `base` é o diretório raiz (examples/), `dir` é o diretório atual.
/// O nome do snapshot é o caminho relativo a `base` sem extensão.
fn collect_kata_recursive(
    base: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(String, String)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Pula subdiretório legacy/ — exemplos de Kata4 com sintaxe
            // que não funciona em Kata5. Não são entrypoints válidos.
            if path.file_name().is_some_and(|n| n == "legacy") {
                continue;
            }
            collect_kata_recursive(base, &path, out);
        } else if path.extension().is_some_and(|ext| ext == "kata") {
            // Nome do snapshot: caminho relativo a examples/ sem extensão.
            let rel = path.strip_prefix(base).unwrap_or(&path).with_extension("");
            let snap_name = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "__");
            out.push((snap_name, path.to_string_lossy().to_string()));
        }
    }
}

#[test]
fn snapshot_exemplos_kata() {
    let files = example_files();
    assert!(
        !files.is_empty(),
        "deve encontrar pelo menos 1 exemplo .kata"
    );

    for (name, file) in &files {
        let output = run_kata(file);
        // Pula arquivos que não são entrypoints (ex: módulos sem main!()).
        // Esses produzem erro `<entry point>` — não são testáveis via snapshot.
        if output.contains("<entry point>") {
            continue;
        }
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_suffix(name);
        settings.bind(|| {
            insta::assert_snapshot!(output);
        });
    }
}
