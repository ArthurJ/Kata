//! Testes E2E — export/import de constants entre módulos.
//!
//! DoD do PRD-constant: módulo A exporta `constant escala := 2`;
//! módulo B importa e usa `escala` dentro de uma action.

use std::fs;
use std::process::Command;

fn kata_bin() -> String {
    std::env::var("KATA_BIN").unwrap_or_else(|_| {
        let manifest = env!("CARGO_MANIFEST_DIR");
        format!("{manifest}/../../target/debug/kata")
    })
}

/// Cria um arquivo `.kata` temporário e retorna o path.
fn write_temp_kata(dir: &std::path::Path, name: &str, content: &str) -> String {
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata run <path>` e retorna (stdout, stderr, exit_code).
fn run_kata(path: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .arg("run")
        .arg(path)
        .output()
        .expect("executar kata run");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// DoD: módulo A exporta `constant escala := 2`; módulo B importa e
/// usa `escala` dentro de uma action.
#[test]
fn constant_export_import_em_action() {
    let dir = std::env::temp_dir().join("kata-driver-constant-fase4-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");

    // Módulo exportador: define e exporta a constant
    let _ = write_temp_kata(
        &dir,
        "mod_a",
        r#"constant escala := 2

export escala"#,
    );

    // Módulo importador: importa e usa a constant em uma action
    let main_path = write_temp_kata(
        &dir,
        "mod_b",
        r#"import mod_a.(escala)

action main => Int
    * escala 21

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&main_path);
    assert_eq!(code, 0, "exit code não-zero. stderr: {stderr}");
    assert!(
        stdout.trim().contains("42"),
        "esperava '42' no stdout. stdout: {stdout}"
    );
}

/// Constant exportada usada no entry point (não em action).
#[test]
fn constant_export_import_em_entry_point() {
    let dir = std::env::temp_dir().join("kata-driver-constant-fase4-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");

    let _ = write_temp_kata(
        &dir,
        "mod_c",
        r#"constant base := 10

export base"#,
    );

    let main_path = write_temp_kata(
        &dir,
        "mod_d",
        r#"import mod_c.(base)

+ base 5"#,
    );

    let (stdout, stderr, code) = run_kata(&main_path);
    assert_eq!(code, 0, "exit code não-zero. stderr: {stderr}");
    assert!(
        stdout.trim().contains("15"),
        "esperava '15' no stdout. stdout: {stdout}"
    );
}

/// Constant não-exportada NÃO é visível para o importador.
#[test]
fn constant_nao_exportada_nao_visivel() {
    let dir = std::env::temp_dir().join("kata-driver-constant-fase4-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");

    let _ = write_temp_kata(
        &dir,
        "mod_e",
        r#"constant secreta := 99

export nada"#,
    );

    // Módulo importador tenta usar `secreta` — deve falhar (não-exportada)
    let main_path = write_temp_kata(
        &dir,
        "mod_f",
        r#"import mod_e.(secreta)

secreta"#,
    );

    let (_stdout, _stderr, code) = run_kata(&main_path);
    assert_ne!(
        code, 0,
        "deveria falhar: constant não-exportada não deve ser importável"
    );
}
