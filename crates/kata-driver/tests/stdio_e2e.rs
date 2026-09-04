//! Testes E2E do PRD-stdio-alignment — stdio como File.
//!
//! Cobertura:
//!  1. stdio_file_stdin          — read!(__stdin__) lê de stdin via File
//!  2. stdio_file_stdout         — echo!(msg, __stdout__) escreve em stdout
//!  3. stdio_file_stderr         — echo!(msg, __stderr__) escreve em stderr
//!  4. stdio_file_close_noop     — close!(__stdout__) é no-op, não crash
//!  5. stdio_stdin_write_erro    — write!(__stdin__, "msg") retorna Err
//!  6. stdio_stdout_read_erro    — read!(__stdout__) retorna Err
//!  7. fiber_fecha_arquivo_sem_close — Fase 9: registry por-fiber

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

/// Executa `kata run <path>` com stdin via piped input.
fn run_kata_with_stdin(path: &str, input: &str) -> (String, String, i32) {
    use std::io::Write;
    let mut child = Command::new(kata_bin())
        .args(["run", path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawnar kata run");
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).expect("escrever stdin");
    }
    let output = child.wait_with_output().expect("esperar kata run");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ── 1. stdio_file_stdin — read!(__stdin__) lê de stdin via File ──

/// `read!(__stdin__)` lê uma linha de stdin via File (FD 0).
/// O programa recebe "hello\n" via stdin e imprime o conteúdo lido.
#[test]
fn stdio_file_stdin() {
    let path = write_temp_kata(
        "stdio_file_stdin",
        r#"import stdio
action main => Int
    let r := readline!(__stdin__)
    match r
        Ok msg: echo!(msg, __stdout__)
        Err _: echo!("erro", __stdout__)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata_with_stdin(&path, "hello\n");
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("hello"),
        "deve imprimir 'hello' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 2. stdio_file_stdout — echo!(msg, __stdout__) escreve em stdout ──

/// `echo!(msg, __stdout__)` escreve a mensagem em stdout via File (FD 1).
#[test]
fn stdio_file_stdout() {
    let path = write_temp_kata(
        "stdio_file_stdout",
        r#"import stdio
action main => Int
    echo!("via-stdout-file", __stdout__)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("via-stdout-file"),
        "deve imprimir 'via-stdout-file' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 3. stdio_file_stderr — echo!(msg, __stderr__) escreve em stderr ──

/// `echo!(msg, __stderr__)` escreve a mensagem em stderr via File (FD 2).
#[test]
fn stdio_file_stderr() {
    let path = write_temp_kata(
        "stdio_file_stderr",
        r#"import stdio
action main => Int
    echo!("via-stderr-file", __stderr__)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stderr.contains("via-stderr-file"),
        "deve imprimir 'via-stderr-file' em stderr — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 4. stdio_file_close_noop — close!(__stdout__) é no-op ──

/// `close!(__stdout__)` não fecha FD 1 — stdout continua funcionando após close.
#[test]
fn stdio_file_close_noop() {
    let path = write_temp_kata(
        "stdio_file_close_noop",
        r#"import stdio
action main => Int
    close!(__stdout__)
    echo!("apos-close", __stdout__)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("apos-close"),
        "stdout deve funcionar após close! (no-op) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 5. stdio_stdin_write_erro — write!(__stdin__, ...) retorna Err ──

/// `write!(__stdin__, "msg")` retorna `Err("not writable")` — stdin é read-only.
#[test]
fn stdio_stdin_write_erro() {
    let path = write_temp_kata(
        "stdio_stdin_write_erro",
        r#"import stdio
action main => Int
    let r := write!(__stdin__, "msg")
    match r
        Ok _: echo!("ok", __stdout__)
        Err e: echo!(e, __stdout__)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("not writable"),
        "deve imprimir 'not writable' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 6. stdio_stdout_read_erro — read!(__stdout__) retorna Err ──

/// `read!(__stdout__)` retorna `Err("not readable")` — stdout é write-only.
#[test]
fn stdio_stdout_read_erro() {
    let path = write_temp_kata(
        "stdio_stdout_read_erro",
        r#"import stdio
action main => Int
    let r := read!(__stdout__)
    match r
        Ok _: echo!("ok", __stdout__)
        Err e: echo!(e, __stdout__)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("not readable"),
        "deve imprimir 'not readable' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 7. fiber_fecha_arquivo_sem_close — Fase 9: registry por-fiber ──

/// Fiber abre arquivo sem chamar `close!()`. Quando o fiber termina,
/// `try_destroy` fecha automaticamente o FD. O arquivo pode ser lido
/// depois — o conteúdo foi escrito e flushed pelo close automático.
#[test]
fn fiber_fecha_arquivo_sem_close() {
    let path = write_temp_kata(
        "fiber_fecha_arquivo_sem_close",
        r#"import stdio
action vazador => Unit
    let f := open!("/tmp/kata-driver-stdio-log-e2e/fase9_vazamento.txt", FileMode::Write)
    match f
        Ok fh:
            write!(fh, "auto-fechado")
            # NÃO chama close!(fh) — try_destroy fecha automaticamente
            ()
        Err _: ()

action main => Int
    fork!(vazador, ())
    sleep!(100)
    # Após o fiber terminar, try_destroy fechou o arquivo.
    # Verificamos lendo o conteúdo de volta.
    let f2 := open!("/tmp/kata-driver-stdio-log-e2e/fase9_vazamento.txt", FileMode::Read)
    match f2
        Ok fh:
            let r := readline!(fh)
            match r
                Ok txt: echo!(txt, __stdout__)
                Err _: echo!("erro-read", __stdout__)
            close!(fh)
        Err _: echo!("erro-open2", __stdout__)
    0

main!()"#,
    );

    // Limpa arquivo de saída anterior
    let _ = fs::remove_file("/tmp/kata-driver-stdio-log-e2e/fase9_vazamento.txt");

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("auto-fechado"),
        "deve imprimir 'auto-fechado' (FD foi fechado por try_destroy) — stdout: {stdout} | stderr: {stderr}"
    );
}
