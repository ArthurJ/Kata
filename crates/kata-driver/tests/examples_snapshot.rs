//! Snapshot tests dos exemplos `.kata` — roda `kata run` e compara stdout.
//!
//! Cada exemplo em `examples/<categoria>/*.kata` é executado via subprocesso
//! `kata run`. O stdout é capturado e comparado com um snapshot insta.
//!
//! Um `#[test]` por categoria permite rodar apenas um tema:
//!   cargo test --test examples_snapshot -- types
//!   cargo test --test examples_snapshot -- concurrency
//!
//! Sem filtro, `cargo test --test examples_snapshot` roda todas as categorias.
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

/// Lista todos os arquivos `.kata` num subdiretório de examples/.
///
/// Retorna (snap_name, path_absoluto) para cada arquivo.
/// Arquivos que não são entrypoints (sem `main!()` ou expressão top-level)
/// são pulados se produzirem erro de `<entry point>`.
fn category_files(category: &str) -> Vec<(String, String)> {
    let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let cat_dir = examples_dir.join(category);
    let mut files: Vec<(String, String)> = Vec::new();
    collect_kata_recursive(&examples_dir, &cat_dir, &mut files);
    files.sort();
    files
}

/// Coleta arquivos `.kata` recursivamente.
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
            collect_kata_recursive(base, &path, out);
        } else if path.extension().is_some_and(|ext| ext == "kata") {
            let rel = path.strip_prefix(base).unwrap_or(&path).with_extension("");
            let snap_name = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "__");
            out.push((snap_name, path.to_string_lossy().to_string()));
        }
    }
}

/// Roda snapshot test para todos os arquivos de uma categoria.
fn run_category(category: &str) {
    let files = category_files(category);
    assert!(
        !files.is_empty(),
        "deve encontrar pelo menos 1 exemplo .kata em {category}/"
    );

    for (name, file) in &files {
        let output = run_kata(file);
        // Pula arquivos que não são entrypoints (ex: módulos sem main!()).
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

#[test]
fn snapshot_actions() {
    run_category("actions");
}

#[test]
fn snapshot_algorithms() {
    run_category("algorithms");
}

#[test]
fn snapshot_basics() {
    run_category("basics");
}

#[test]
fn snapshot_collections() {
    run_category("collections");
}

#[test]
fn snapshot_concurrency() {
    run_category("concurrency");
}

#[test]
fn snapshot_control_flow() {
    run_category("control_flow");
}

#[test]
fn snapshot_directives() {
    run_category("directives");
}

#[test]
fn snapshot_functions() {
    run_category("functions");
}

#[test]
fn snapshot_imports() {
    run_category("imports");
}

#[test]
fn snapshot_types() {
    run_category("types");
}
