//! Testes E2E — Diretivas customizadas: importação de diretivas entre módulos.
//!
//! Valida que uma diretiva exportada de um módulo pode ser importada e aplicada
//! em outro módulo, com o driver resolvendo o import e mesclando o DirectiveRegistry.

use std::fs;
use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

// ── Test 20: Importação — diretiva exportada de outro módulo ─────────
//
// Cria dois arquivos: tracing_mod.kata (exportador) e main_import.kata
// (importador). O importador usa `import tracing_mod.trace_enter` e aplica
// `@trace_enter` numa action. O driver resolve o import, carrega o módulo
// exportador, mescla o DirectiveRegistry, e o desugaring inlinea a diretiva.

#[test]
fn e2e_import_directive() {
    let dir = std::env::temp_dir().join("kata-driver-directives-e2e-import");
    fs::create_dir_all(&dir).expect("criar temp dir");

    // Módulo exportador: define e exporta a diretiva
    let mod_path = dir.join("tracing_mod.kata");
    fs::write(
        &mod_path,
        r#"directive trace_enter{when: Hook::Enter, on: Target::Action}
    echo!("imported-enter")

export trace_enter"#,
    )
    .expect("escrever modulo exportador");

    // Módulo importador: importa e aplica a diretiva
    let main_path = dir.join("main_import.kata");
    fs::write(
        &main_path,
        r#"import tracing_mod.(trace_enter)

@trace_enter
action greet(name :: Text) => Unit
    echo!("hello")

greet!("world")"#,
    )
    .expect("escrever modulo importador");

    let output = Command::new(kata_bin())
        .args(["run", main_path.to_str().unwrap()])
        .output()
        .expect("executar kata run");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("imported-enter"),
        "deve imprimir imported-enter (diretiva importada) - stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("hello"),
        "deve imprimir hello (body original) - stdout: {stdout} | stderr: {stderr}"
    );
}
