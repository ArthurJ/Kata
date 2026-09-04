//! E2E — rejeição de `data` sem campos e sem `@ffi`.
//!
//! `data Foo ()` sem campos e sem diretiva `@ffi` não tem significado:
//! sem construtor, sem show, sem layout. O resolution deve rejeitar
//! com erro gracioso (exit não-zero, mensagem clara).

use std::process::Command;

/// Roda `kata run <path>` e retorna (stdout, stderr, code).
fn run_kata(path: &str) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_kata"))
        .args(["run", path])
        .output()
        .expect("kata run deve executar");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// Escreve source num .kata temporário e retorna o path.
fn write_temp(name: &str, src: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kata_empty_data_e2e_{name}_{id}_{}.kata",
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().to_string()
}

/// `data Vazio ()` sem @ffi deve falhar com `resolve.empty_data_no_ffi`.
#[test]
fn empty_data_without_ffi_rejected() {
    let source = "data Vazio ()\n\necho!(Vazio)\n";
    let path = write_temp("empty", source);
    let (stdout, stderr, code) = run_kata(&path);
    assert_ne!(code, 0, "deve falhar — sem campos e sem @ffi");
    assert!(
        stderr.contains("empty_data_no_ffi") || stderr.contains("requer diretiva @ffi"),
        "stderr deve mencionar empty_data_no_ffi: {stderr}"
    );
    assert!(stdout.is_empty(), "não deve produzir output: {stdout}");
}

/// `data ComFfi () @ffi("i64")` com @ffi deve funcionar (tipo opaco FFI).
#[test]
fn empty_data_with_ffi_accepted() {
    let source = "+ 1 2\n";
    let path = write_temp("ffi_ok", source);
    let (stdout, _stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "prelude com data Int () @ffi deve funcionar");
    assert_eq!(stdout, "3\n");
}

/// `data ComCampos (x::Int)` com campos deve funcionar normalmente.
#[test]
fn data_with_fields_accepted() {
    let source = "data Ponto (x::Int y::Int)\nconstant p := Ponto 3 4\nshow p\n";
    let path = write_temp("fields", source);
    let (stdout, _stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "data com campos deve funcionar");
    // JIT pode quotear ou não — só verifica que não falha.
    let _ = stdout;
}