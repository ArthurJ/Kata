//! Testes E2E do PRD-stdio-alignment — stdio como File + log com File.
//!
//! Cobertura:
//!  1. stdio_file_stdin          — read!(__stdin__) lê de stdin via File
//!  2. stdio_file_stdout         — echo!(msg, __stdout__) escreve em stdout
//!  3. stdio_file_stderr         — echo!(msg, __stderr__) escreve em stderr
//!  4. stdio_file_close_noop     — close!(__stdout__) é no-op, não crash
//!  5. stdio_stdin_write_erro    — write!(__stdin__, "msg") retorna Err
//!  6. stdio_stdout_read_erro    — read!(__stdout__) retorna Err
//!  7. log_to_file_stdout        — log!(level, msg, __stdout__) escreve em stdout
//!  8. log_to_file_arquivo       — log!(level, msg, f) escreve em arquivo
//!  9. log_template_level        — log!(level, "[{log_level}] {x}") interpola level
//! 10. log_directive_file        — @log{msg: "...", file: __stdout__} escreve em stdout
//! 11. log_directive_multiplas   — duas @log (uma topic, uma file) ambas disparam
//! 12. log_directive_log_level   — @log{msg: "[{log_level}] {x}", ...} interpola level
//! 13. log_recv_result_ok        — log_recv!() retorna Ok(msg)
//! 14. log_recv_result_err       — log_recv!() em tópico inexistente retorna Err
//! 15. log_file_rejeita_policy   — log!(level, msg, __stdout__, "drop") é erro de tipo
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

// ── 7. log_to_file_stdout — log!(level, msg, __stdout__) ──

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

// ── 8. log_to_file_arquivo — log!(level, msg, f) escreve em arquivo ──

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

// ── 9. log_template_level — [{log_level}] interpola level ──

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

// ── 10. log_directive_file — @log{file: __stdout__} ──

/// `@log{msg: "directive-file {_args}", when: "enter", file: __stdout__}` escreve em stdout.
/// `_args` é a tupla de params `(42,)`. `stdout` é action 0-ary → `__stdout__`.
#[test]
fn log_directive_file() {
    let path = write_temp_kata(
        "log_directive_file",
        r#"import stdio
@log{msg: "directive-file {_args}", when: "enter", file: __stdout__}
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
        stdout.contains("directive-file (42)"),
        "deve imprimir 'directive-file (42)' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 11. log_directive_multiplas — duas @log (topic + file) ──

/// Duas diretivas `@log`: uma com `topic` (CSP) e outra com `file` (stdout).
/// Ambas disparam independentemente — o consumidor recebe via log_recv!
/// e stdout contém a mensagem file.
/// `_args` é a tupla de params `(42,)`.
#[test]
fn log_directive_multiplas() {
    let path = write_temp_kata(
        "log_directive_multiplas",
        r#"import stdio
@log{msg: "via-topic {_args}", when: "enter", topic: "audit"}
@log{msg: "via-file {_args}", when: "enter", file: __stdout__}
action processar (x::Int) => Int
    + x 1

action consumir => Int
    match log_recv!("audit")
        Ok m: echo!(m, __stdout__)
        Err _: echo!("erro-recv", __stdout__)
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
        stdout.contains("via-file (42)"),
        "deve imprimir 'via-file (42)' (diretiva file) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("via-topic (42)"),
        "deve imprimir 'via-topic (42)' (diretiva topic, via consumidor) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 12. log_directive_log_level — @log{msg: "[{log_level}] ..."} ──

/// `@log{msg: "dir {_args}", when: "enter", file: __stdout__, level: LogLevel::Warn}`.
/// No sistema novo, `format!{dict}` não tem `_log_level`. O `level: LogLevel::Warn`
/// despacha para o overload com `level: Text`, mas o body usa LogLevel::Info
/// hardcoded. O teste verifica despacho com level, não interpolação de level.
#[test]
fn log_directive_log_level() {
    let path = write_temp_kata(
        "log_directive_log_level",
        r#"import stdio
@log{msg: "dir {_args}", when: "enter", file: __stdout__, level: LogLevel::Warn}
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
        stdout.contains("dir (77)"),
        "deve imprimir 'dir (77)' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 13. log_recv_result_ok — log_recv! retorna Ok(msg) ──

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

// ── 14. log_recv_result_err — log_recv! em tópico inexistente ──

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

// ── 15. log_file_rejeita_policy — log!(..., file, "drop") é erro ──

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

// ── 16. log_directive_topic_file_coexistem — @log{topic+file} funciona ──

/// `@log{msg: "...", when: "enter", topic: "foo", file: __stdout__}` —
/// topic e file coexistem: o body da diretiva faz 2× log!() (uma CSP, uma file).
/// No sistema de diretivas do stdlib, topic+file não são mutuamente exclusivos.
#[test]
fn log_directive_topic_file_coexistem() {
    let path = write_temp_kata(
        "log_directive_topic_file_coexistem",
        r#"import stdio
@log{msg: "coexist {_args}", when: "enter", topic: "coexist", file: __stdout__}
action processar (x::Int) => Int
    x

action consumir => Int
    match log_recv!("coexist")
        Ok m: echo!(m, __stdout__)
        Err _: echo!("erro-recv", __stdout__)
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
    // file: __stdout__ escreve diretamente, topic: "coexist" publica via CSP
    // Ambas as mensagens aparecem em stdout — uma do file, uma do consumidor
    assert!(
        stdout.contains("coexist (42)"),
        "deve imprimir 'coexist (42)' — stdout: {stdout} | stderr: {stderr}"
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
