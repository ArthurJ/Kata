//! Testes E2E do PRD-stdio-alignment — stdio como File + log com File.
//!
//! Cobertura:
//!  1. stdio_file_stdin          — read!(stdin!()) lê de stdin via File
//!  2. stdio_file_stdout         — echo!(msg, stdout!()) escreve em stdout
//!  3. stdio_file_stderr         — echo!(msg, stderr!()) escreve em stderr
//!  4. stdio_file_close_noop     — close!(stdout!()) é no-op, não crash
//!  5. stdio_stdin_write_erro    — write!(stdin!(), "msg") retorna Err
//!  6. stdio_stdout_read_erro    — read!(stdout!()) retorna Err
//!  7. log_to_file_stdout        — log!(level, msg, stdout!()) escreve em stdout
//!  8. log_to_file_arquivo       — log!(level, msg, f) escreve em arquivo
//!  9. log_template_level        — log!(level, "[{log_level}] {x}") interpola level
//! 10. log_directive_file        — @log{msg: "...", file: stdout} escreve em stdout
//! 11. log_directive_multiplas   — duas @log (uma topic, uma file) ambas disparam
//! 12. log_directive_log_level   — @log{msg: "[{log_level}] {x}", ...} interpola level
//! 13. log_recv_result_ok        — log_recv!() retorna Ok(msg)
//! 14. log_recv_result_err       — log_recv!() em tópico inexistente retorna Err
//! 15. log_file_rejeita_policy   — log!(level, msg, stdout!(), "drop") é erro de tipo
//! 16. log_directive_topic_file_exclusivos — @log{topic: ..., file: ...} é erro

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

// ── 1. stdio_file_stdin — read!(stdin!()) lê de stdin via File ──

/// `read!(stdin!())` lê uma linha de stdin via File (FD 0).
/// O programa recebe "hello\n" via stdin e imprime o conteúdo lido.
#[test]
fn stdio_file_stdin() {
    let path = write_temp_kata(
        "stdio_file_stdin",
        r#"import stdio.(stdin, stdout)
action main => Int
    let r := readline!(stdin!())
    match r
        Result::Ok msg: echo!(msg, stdout!())
        Result::Err _: echo!("erro", stdout!())
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

// ── 2. stdio_file_stdout — echo!(msg, stdout!()) escreve em stdout ──

/// `echo!(msg, stdout!())` escreve a mensagem em stdout via File (FD 1).
#[test]
fn stdio_file_stdout() {
    let path = write_temp_kata(
        "stdio_file_stdout",
        r#"import stdio.(stdout)
action main => Int
    echo!("via-stdout-file", stdout!())
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

// ── 3. stdio_file_stderr — echo!(msg, stderr!()) escreve em stderr ──

/// `echo!(msg, stderr!())` escreve a mensagem em stderr via File (FD 2).
#[test]
fn stdio_file_stderr() {
    let path = write_temp_kata(
        "stdio_file_stderr",
        r#"import stdio.(stderr)
action main => Int
    echo!("via-stderr-file", stderr!())
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

// ── 4. stdio_file_close_noop — close!(stdout!()) é no-op ──

/// `close!(stdout!())` não fecha FD 1 — stdout continua funcionando após close.
#[test]
fn stdio_file_close_noop() {
    let path = write_temp_kata(
        "stdio_file_close_noop",
        r#"import stdio.(stdout)
action main => Int
    close!(stdout!())
    echo!("apos-close", stdout!())
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

// ── 5. stdio_stdin_write_erro — write!(stdin!(), ...) retorna Err ──

/// `write!(stdin!(), "msg")` retorna `Err("not writable")` — stdin é read-only.
#[test]
fn stdio_stdin_write_erro() {
    let path = write_temp_kata(
        "stdio_stdin_write_erro",
        r#"import stdio.(stdin, stdout)
action main => Int
    let r := write!(stdin!(), "msg")
    match r
        Result::Ok _: echo!("ok", stdout!())
        Result::Err e: echo!(e, stdout!())
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

// ── 6. stdio_stdout_read_erro — read!(stdout!()) retorna Err ──

/// `read!(stdout!())` retorna `Err("not readable")` — stdout é write-only.
#[test]
fn stdio_stdout_read_erro() {
    let path = write_temp_kata(
        "stdio_stdout_read_erro",
        r#"import stdio.(stdout)
action main => Int
    let r := read!(stdout!())
    match r
        Result::Ok _: echo!("ok", stdout!())
        Result::Err e: echo!(e, stdout!())
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

// ── 7. log_to_file_stdout — log!(level, msg, stdout!()) ──

/// `log!(LogLevel::Info, "msg {x}", stdout!())` escreve em stdout via File.
#[test]
fn log_to_file_stdout() {
    let path = write_temp_kata(
        "log_to_file_stdout",
        r#"import stdio.(stdout)
action main => Int
    let x := 42
    log!(LogLevel::Info, "log-msg {x}", stdout!())
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

// ── 8. log_to_file_arquivo — log!(level, msg, f) escreve em arquivo ──

/// `log!(LogLevel::Info, "msg {x}", f)` escreve em arquivo aberto.
/// O arquivo é criado, escrito via log!(), lido de volta e verificado.
#[test]
fn log_to_file_arquivo() {
    let path = write_temp_kata(
        "log_to_file_arquivo",
        r#"import stdio.(stdout)
action main => Int
    let f := open!("/tmp/kata-driver-stdio-log-e2e/log_test_out.txt", FileMode::Write)
    match f
        Result::Ok fh:
            log!(LogLevel::Info, "arquivo-log", fh)
            close!(fh)
        Result::Err _: echo!("erro-open", stdout!())
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

// ── 9. log_template_level — [{log_level}] interpola level ──

/// `log!(LogLevel::Warn, "[{log_level}] {x}")` interpola {log_level} como "Warn".
#[test]
fn log_template_level() {
    let path = write_temp_kata(
        "log_template_level",
        r#"import stdio.(stdout)
action main => Int
    let x := 99
    log!(LogLevel::Warn, "[{log_level}] val={x}", stdout!())
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

// ── 10. log_directive_file — @log{file: stdout} ──

/// `@log{msg: "entrada {x}", when: "enter", file: stdout}` escreve em stdout.
/// `stdout` é action 0-ary → o inference gera `stdout!()` como expressão File.
#[test]
fn log_directive_file() {
    let path = write_temp_kata(
        "log_directive_file",
        r#"import stdio.(stdout)
@log{msg: "directive-file {x}", when: "enter", file: stdout}
action processar (x::Int) => Int
    + x 1

action main => Int
    processar!(42)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("directive-file 42"),
        "deve imprimir 'directive-file 42' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 11. log_directive_multiplas — duas @log (topic + file) ──

/// Duas diretivas `@log`: uma com `topic` (CSP) e outra com `file` (stdout).
/// Ambas disparam independentemente — o consumidor recebe via log_recv!
/// e stdout contém a mensagem file.
#[test]
fn log_directive_multiplas() {
    let path = write_temp_kata(
        "log_directive_multiplas",
        r#"import stdio.(stdout)
@log{msg: "via-topic {x}", when: "enter", topic: "audit"}
@log{msg: "via-file {x}", when: "enter", file: stdout}
action processar (x::Int) => Int
    + x 1

action consumir => Int
    match log_recv!("audit")
        Result::Ok m: echo!(m, stdout!())
        Result::Err _: echo!("erro-recv", stdout!())
    0

action main => Int
    fork!(processar, (42))
    fork!(consumir, ())
    sleep!(50)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("via-file 42"),
        "deve imprimir 'via-file 42' (diretiva file) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("via-topic 42"),
        "deve imprimir 'via-topic 42' (diretiva topic, via consumidor) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 12. log_directive_log_level — @log{msg: "[{log_level}] ..."} ──

/// `@log{msg: "[{log_level}] {x}", when: "enter", file: stdout, level: "Warn"}`
/// interpola {log_level} como "Warn".
#[test]
fn log_directive_log_level() {
    let path = write_temp_kata(
        "log_directive_log_level",
        r#"import stdio.(stdout)
@log{msg: "[{log_level}] dir {x}", when: "enter", file: stdout, level: "Warn"}
action processar (x::Int) => Int
    x

action main => Int
    processar!(77)
    0

main!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("[Warn] dir 77"),
        "deve imprimir '[Warn] dir 77' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 13. log_recv_result_ok — log_recv! retorna Ok(msg) ──

/// `log_recv!("topic")` com mensagem disponível retorna `Ok(msg)`.
/// O match em `Result::Ok` extrai a mensagem.
#[test]
fn log_recv_result_ok() {
    let path = write_temp_kata(
        "log_recv_result_ok",
        r#"import stdio.(stdout)
action emitir => Unit
    log!(LogLevel::Info, "msg-ok", "test-ok")
    ()

action consumir => Int
    match log_recv!("test-ok")
        Result::Ok m: echo!(m, stdout!())
        Result::Err e: echo!("err: {e}", stdout!())
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

// ── 14. log_recv_result_err — log_recv! em tópico inexistente ──

/// `log_recv!("inexistente")` em tópico sem publicador retorna `Err`.
#[test]
fn log_recv_result_err() {
    let path = write_temp_kata(
        "log_recv_result_err",
        r#"import stdio.(stdout)
action consumir => Int
    match log_recv!("topico-inexistente")
        Result::Ok m: echo!("ok: {m}", stdout!())
        Result::Err e: echo!("err: {e}", stdout!())
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

// ── 15. log_file_rejeita_policy — log!(..., file, "drop") é erro ──

/// `log!(level, msg, stdout!(), "drop")` — policy com File é erro de tipo.
/// O 4º argumento (policy) não é válido quando o 3º é File.
#[test]
fn log_file_rejeita_policy() {
    let path = write_temp_kata(
        "log_file_rejeita_policy",
        r#"import stdio.(stdout)
action main => Int
    log!(LogLevel::Info, "msg", stdout!(), "drop")
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

// ── 16. log_directive_topic_file_exclusivos — @log{topic+file} é erro ──

/// `@log{msg: "...", when: "enter", topic: "foo", file: stdout}` —
/// topic e file são mutuamente exclusivos. Erro de resolution.
#[test]
fn log_directive_topic_file_exclusivos() {
    let path = write_temp_kata(
        "log_directive_topic_file_exclusivos",
        r#"import stdio.(stdout)
@log{msg: "test", when: "enter", topic: "foo", file: stdout}
action processar (x::Int) => Int
    x

action main => Int
    processar!(42)
    0

main!()"#,
    );

    let (_stdout, stderr, code) = run_kata(&path);
    assert_ne!(
        code, 0,
        "deve falhar (topic+file exclusivos) — stderr: {stderr}"
    );
    assert!(
        stderr.contains("mutuamente exclusivos") || stderr.contains("exclusivos"),
        "erro deve mencionar exclusividade topic/file — stderr: {stderr}"
    );
}

// ── 17. fiber_fecha_arquivo_sem_close — Fase 9: registry por-fiber ──

/// Fiber abre arquivo sem chamar `close!()`. Quando o fiber termina,
/// `try_destroy` fecha automaticamente o FD. O arquivo pode ser lido
/// depois — o conteúdo foi escrito e flushed pelo close automático.
#[test]
fn fiber_fecha_arquivo_sem_close() {
    let path = write_temp_kata(
        "fiber_fecha_arquivo_sem_close",
        r#"import stdio.(stdout)
action vazador => Unit
    let f := open!("/tmp/kata-driver-stdio-log-e2e/fase9_vazamento.txt", FileMode::Write)
    match f
        Result::Ok fh:
            write!(fh, "auto-fechado")
            # NÃO chama close!(fh) — try_destroy fecha automaticamente
            ()
        Result::Err _: ()

action main => Int
    fork!(vazador, ())
    sleep!(100)
    # Após o fiber terminar, try_destroy fechou o arquivo.
    # Verificamos lendo o conteúdo de volta.
    let f2 := open!("/tmp/kata-driver-stdio-log-e2e/fase9_vazamento.txt", FileMode::Read)
    match f2
        Result::Ok fh:
            let r := readline!(fh)
            match r
                Result::Ok txt: echo!(txt, stdout!())
                Result::Err _: echo!("erro-read", stdout!())
            close!(fh)
        Result::Err _: echo!("erro-open2", stdout!())
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
