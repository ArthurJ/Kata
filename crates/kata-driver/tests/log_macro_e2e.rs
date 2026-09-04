//! Testes E2E do PRD-stdio-alignment — log!() macro e log_recv!() com File.
//!
//! Cobertura:
//!  1. log_to_file_stdout        — log!(level, msg, __stdout__) escreve em stdout
//!  2. log_to_file_arquivo       — log!(level, msg, f) escreve em arquivo
//!  3. log_template_level        — log!(level, "[{log_level}] {x}") interpola level
//!  4. log_recv_result_ok        — log_recv!() retorna Ok(msg)
//!  5. log_recv_result_err       — log_recv!() em tópico inexistente retorna Err
//!  6. log_file_rejeita_policy   — log!(level, msg, __stdout__, "drop") é erro de tipo

use std::fs;
use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Cria um arquivo `.kata` temporário e retorna o path.
fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-stdio-log-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata run <path>` e retorna (stdout, stderr, exit_code).
fn run_kata(path: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .args(["run", path])
        .output()
        .expect("executar kata run");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ── 1. log_to_file_stdout — log!(level, msg, __stdout__) ──

/// `log!(LogLevel::Info, "msg {x}", __stdout__)` escreve em stdout via File.
#[test]
fn log_to_file_stdout() {
    let path = write_temp_kata(
        "log_to_file_stdout",
        r#"import stdio
action main => Int
    let x := 42
    let msg := + "log-msg " (show x)
    log!(LogLevel::Info, msg, __stdout__)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("log-msg 42"),
        "deve imprimir 'log-msg 42' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 2. log_to_file_arquivo — log!(level, msg, f) escreve em arquivo ──

/// `log!(LogLevel::Info, "msg {x}", f)` escreve em arquivo aberto.
/// O arquivo é criado, escrito via log!(), lido de volta e verificado.
#[test]
fn log_to_file_arquivo() {
    let path = write_temp_kata(
        "log_to_file_arquivo",
        r#"import stdio
action main => Int
    let f := open!("/tmp/kata-driver-stdio-log-e2e/log_test_out.txt", FileMode::Write)
    match f
        Ok fh:
            log!(LogLevel::Info, "arquivo-log", fh)
            close!(fh)
        Err _: echo!("erro-open", __stdout__)
    0

main!()"#,
    );

    // Limpa arquivo de saída anterior
    let _ = fs::remove_file("/tmp/kata-driver-stdio-log-e2e/log_test_out.txt");

    let (_stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    let content =
        fs::read_to_string("/tmp/kata-driver-stdio-log-e2e/log_test_out.txt").unwrap_or_default();
    assert!(
        content.contains("arquivo-log"),
        "arquivo deve conter 'arquivo-log' — content: {content} | stderr: {stderr}"
    );
}

// ── 3. log_template_level — [{log_level}] interpola level ──

/// `log!(LogLevel::Warn, "[{log_level}] {x}")` interpola {log_level} como "Warn".
#[test]
fn log_template_level() {
    let path = write_temp_kata(
        "log_template_level",
        r#"import stdio
action main => Int
    let x := 99
    let msg := + "[Warn] val=" (show x)
    log!(LogLevel::Warn, msg, __stdout__)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("[Warn] val=99"),
        "deve imprimir '[Warn] val=99' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 4. log_recv_result_ok — log_recv! retorna Ok(msg) ──

/// `log_recv!("topic")` com mensagem disponível retorna `Ok(msg)`.
/// O match em `Ok` extrai a mensagem.
#[test]
fn log_recv_result_ok() {
    let path = write_temp_kata(
        "log_recv_result_ok",
        r#"import stdio
action emitir => Unit
    log!(LogLevel::Info, "msg-ok", "test-ok")
    ()

action consumir => Int
    match log_recv!("test-ok")
        Ok m: echo!(m, __stdout__)
        Err e: echo!("err: {e}", __stdout__)
    0

action main => Int
    fork!(emitir, ())
    fork!(consumir, ())
    sleep!(50)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("msg-ok"),
        "deve imprimir 'msg-ok' (Ok) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 5. log_recv_result_err — log_recv! em tópico inexistente ──

/// `log_recv!("inexistente")` em tópico sem publicador retorna `Err`.
#[test]
fn log_recv_result_err() {
    let path = write_temp_kata(
        "log_recv_result_err",
        r#"import stdio
action consumir => Int
    match log_recv!("topico-inexistente")
        Ok m: echo!("ok: {m}", __stdout__)
        Err e: echo!("err: {e}", __stdout__)
    0

action main => Int
    fork!(consumir, ())
    sleep!(50)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("err:"),
        "deve imprimir 'err:...' (Err) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 6. log_file_rejeita_policy — log!(..., file, "drop") é erro ──

/// `log!(level, msg, __stdout__, "drop")` — policy com File é erro de tipo.
/// O 4º argumento (policy) não é válido quando o 3º é File.
#[test]
fn log_file_rejeita_policy() {
    let path = write_temp_kata(
        "log_file_rejeita_policy",
        r#"import stdio
action main => Int
    log!(LogLevel::Info, "msg", __stdout__, "drop")
    0

main!()"#,
    );

    let (_stdout, stderr, code) = run_kata(&path);
    assert_ne!(code, 0, "deve falhar (erro de tipo) — stderr: {stderr}");
    assert!(
        stderr.contains("policy") || stderr.contains("não é válido"),
        "erro deve mencionar policy inválido com File — stderr: {stderr}"
    );
}
