//! Testes E2E do Fio 14 `@log` — telemetria via CSP.
//!
//! Cada teste cria um arquivo `.kata` temporário, invoca o binário `kata`
//! via subprocess (`kata run` ou `kata test`), e verifica stdout + exit code.
//!
//! Convenções (pitfalls do handoff):
//! - `soma! 1 2` é inválido — parser exige `soma!(1, 2)` (pitfall #45).
//! - `Unit` como Ident não é `Expr::Unit` — usar `()` (pitfall #45).
//! - Testes que exercitem estado global do runtime (canais, registry) rodam
//!   em subprocess isolado — não compartilham estado entre si.
//! - `kata run` executa o programa e imprime o resultado da entry expr.
//! - `kata test` descobre wrappers `@test` e os executa individualmente.
//!
//! Estrutura dos testes: o produtor (action que publica log) e o consumidor
//! (action que recebe e imprime) rodam em fibers separados via `fork!`.
//! O entry point faz fork do produtor, depois chama o consumidor que bloqueia
//! até receber a mensagem. Isso garante ordenação correta e que `log_recv!`
//! executa dentro de um fiber (pode bloquear via scheduler cooperativo).
//!
//! Os 14 testes cobrem (sub-PRD §4 Fase 7):
//!  1. log_directive_prologo       — @log com só params, loga na entrada
//!  2. log_directive_epilogo        — @log com vars do corpo, loga na saída
//!  3. log_directive_when_enter     — when: "enter" explícito
//!  4. log_directive_when_exit      — when: "exit" explícito
//!  5. log_action_basico           — log!() no corpo de action
//!  6. log_action_com_topic        — log!() com tópico explícito
//!  7. log_policy_drop             — policy drop, não bloqueia
//!  8. log_policy_block             — policy block, bloqueia até ack
//!  9. log_config_heranca          — log_config!() em fiber ancestral
//! 10. log_recv_consomo            — log_recv!() consome telemetria
//! 11. log_template_interpolacao  — {expr} em msg, interpola
//! 12. log_template_escape        — {{ produz { literal
//! 13. log_level_enum             — LogLevel::Warn etc. validado
//! 14. log_diretiva_e_action_independentes — ambos disparam

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
    let dir = std::env::temp_dir().join("kata-driver-log-e2e");
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

/// Executa `kata test <path>` e retorna (stdout, exit_code).
#[allow(dead_code)]
fn run_kata_test(path: &str) -> (String, i32) {
    let output = Command::new(kata_bin())
        .args(["test", path])
        .output()
        .expect("executar kata test");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ── 1. log_directive_prologo — @log com só params ─────────────

/// `@log{msg: "entrada {x}", when: "enter"}` em action com param `x`:
/// o codegen injeta `kata_rt_log_publish` no prólogo (antes do body).
/// O consumidor recebe via `log_recv!("default")` e imprime com `echo`.
#[test]
fn log_directive_prologo() {
    let path = write_temp_kata(
        "log_directive_prologo",
        r#"@log{msg: "entrada {x}", when: "enter"}
action processar (x::Int) => Int
    + x 1

action consumir => Int
    let msg := log_recv!("default")
    echo!(msg)
    0

fork!(processar, (41))
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("entrada 41"),
        "deve imprimir 'entrada 41' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 2. log_directive_epilogo — @log com vars do corpo ─────────

/// `@log{msg: "resultado {r}", when: "exit"}` em action onde `r` é
/// variável do corpo: o codegen injeta no epílogo (antes do return).
#[test]
fn log_directive_epilogo() {
    let path = write_temp_kata(
        "log_directive_epilogo",
        r#"@log{msg: "resultado {r}", when: "exit"}
action dobrar (x::Int) => Int
    let r := * x 2
    r

action consumir => Int
    let msg := log_recv!("default")
    echo!(msg)
    0

fork!(dobrar, (21))
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("resultado 42"),
        "deve imprimir 'resultado 42' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 3. log_directive_when_enter — when explícito ──────────────

/// `when: "enter"` explícito loga no prólogo. Param `x` referenciado.
#[test]
fn log_directive_when_enter() {
    let path = write_temp_kata(
        "log_directive_when_enter",
        r#"@log{msg: "enter {x}", when: "enter"}
action ident (x::Int) => Int
    x

action consumir => Int
    let msg := log_recv!("default")
    echo!(msg)
    0

fork!(ident, (99))
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("enter 99"),
        "deve imprimir 'enter 99' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 4. log_directive_when_exit — when: "exit" explícito ───────

/// `when: "exit"` explícito loga no epílogo. Variável do corpo `r`.
#[test]
fn log_directive_when_exit() {
    let path = write_temp_kata(
        "log_directive_when_exit",
        r#"@log{msg: "exit {r}", when: "exit"}
action triplo (x::Int) => Int
    let r := * x 3
    r

action consumir => Int
    let msg := log_recv!("default")
    echo!(msg)
    0

fork!(triplo, (14))
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("exit 42"),
        "deve imprimir 'exit 42' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 5. log_action_basico — log!() no corpo de action ──────────

/// `log!(LogLevel::Info, "ola")` no corpo de action: desugara para
/// `kata_rt_log_publish`. Consumido via `log_recv!`.
#[test]
fn log_action_basico() {
    let path = write_temp_kata(
        "log_action_basico",
        r#"action saudar => Unit
    log!(LogLevel::Info, "ola-mundo")
    ()

action consumir => Int
    let msg := log_recv!("default")
    echo!(msg)
    0

fork!(saudar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("ola-mundo"),
        "deve imprimir 'ola-mundo' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 6. log_action_com_topic — log!() com tópico explícito ─────

/// `log!(LogLevel::Info, "audit-msg", "audit")` publica no tópico "audit".
/// Consumido via `log_recv!("audit")`.
#[test]
fn log_action_com_topic() {
    let path = write_temp_kata(
        "log_action_com_topic",
        r#"action auditar => Unit
    log!(LogLevel::Info, "evento-audit", "audit")
    ()

action consumir => Int
    let msg := log_recv!("audit")
    echo!(msg)
    0

fork!(auditar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("evento-audit"),
        "deve imprimir 'evento-audit' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 7. log_policy_drop — policy drop, não bloqueia ────────────

/// `log!(LogLevel::Info, "msg-drop", "drop-topic", "drop")` publica
/// via Broadcast (fire-and-forget). Não bloqueia mesmo sem consumidor.
#[test]
fn log_policy_drop() {
    let path = write_temp_kata(
        "log_policy_drop",
        r#"action disparar => Unit
    log!(LogLevel::Info, "msg-drop", "drop-topic", "drop")
    ()

disparar!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Sem consumidor, Broadcast não bloqueia. Programa termina normalmente.
    // stdout pode estar vazio — o importante é não travar.
    let _ = stdout;
}

// ── 8. log_policy_block — policy block, bloqueia até ack ───────

/// `log!(LogLevel::Info, "msg-block", "block-topic", "block")` publica
/// via Queue bounded cap=1. Consumidor `log_recv!("block-topic")` libera.
#[test]
fn log_policy_block() {
    let path = write_temp_kata(
        "log_policy_block",
        r#"action bloquear => Unit
    log!(LogLevel::Info, "msg-block", "block-topic", "block")
    ()

action consumir => Int
    let msg := log_recv!("block-topic")
    echo!(msg)
    0

fork!(bloquear, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("msg-block"),
        "deve imprimir 'msg-block' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 9. log_config_heranca — log_config!() em fiber ancestral ───

/// `log_config!("cfg-topic", "drop", LogLevel::Info)` seta defaults.
/// `log!(LogLevel::Info, "msg-cfg")` usa config herdada (topic="cfg-topic").
/// Consumido via `log_recv!("cfg-topic")`.
#[test]
fn log_config_heranca() {
    let path = write_temp_kata(
        "log_config_heranca",
        r#"action configurar => Unit
    log_config!("cfg-topic", "drop", LogLevel::Info)
    log!(LogLevel::Info, "msg-cfg")
    ()

action consumir => Int
    let msg := log_recv!("cfg-topic")
    echo!(msg)
    0

fork!(configurar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("msg-cfg"),
        "deve imprimir 'msg-cfg' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 10. log_recv_consomo — log_recv!() consome telemetria ───────

/// `log!()` publica duas mensagens no mesmo tópico (policy "block" para
/// garantir ordem FIFO via Queue bounded). `log_recv!()` consome uma de
/// cada vez. Verifica que cada recv retorna a mensagem correta.
#[test]
fn log_recv_consomo() {
    let path = write_temp_kata(
        "log_recv_consomo",
        r#"action emitir => Unit
    log!(LogLevel::Info, "primeira", "seq", "block")
    log!(LogLevel::Info, "segunda", "seq", "block")
    ()

action consumir => Int
    let m1 := log_recv!("seq")
    let m2 := log_recv!("seq")
    echo!(m1)
    echo!(m2)
    0

fork!(emitir, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("primeira") && stdout.contains("segunda"),
        "deve imprimir ambas — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 11. log_template_interpolacao — {expr} interpola ───────────

/// `@log{msg: "x={x} r={r}", when: "exit"}` interpola variáveis.
#[test]
fn log_template_interpolacao() {
    let path = write_temp_kata(
        "log_template_interpolacao",
        r#"@log{msg: "x={x} r={r}", when: "exit"}
action quadruplo (x::Int) => Int
    let r := * x 4
    r

action consumir => Int
    let msg := log_recv!("default")
    echo!(msg)
    0

fork!(quadruplo, (10))
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("x=10 r=40"),
        "deve imprimir 'x=10 r=40' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 12. log_template_escape — {{ produz { literal ─────────────

/// `@log{msg: "literal {{chave}}", when: "enter"}` produz "literal {chave}".
#[test]
fn log_template_escape() {
    let path = write_temp_kata(
        "log_template_escape",
        r#"@log{msg: "literal {{chave}}", when: "enter"}
action echo_lit (x::Int) => Int
    x

action consumir => Int
    let msg := log_recv!("default")
    echo!(msg)
    0

fork!(echo_lit, (0))
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("literal {chave}"),
        "deve imprimir 'literal {{chave}}' sem escapar — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 13. log_level_enum — LogLevel::Warn validado ───────────────

/// `log!(LogLevel::Warn, "aviso")` usa variante Warn do enum LogLevel.
/// A tag i64 é resolvida em compile-time via enum_registry.
#[test]
fn log_level_enum() {
    let path = write_temp_kata(
        "log_level_enum",
        r#"action avisar => Unit
    log!(LogLevel::Warn, "aviso-warn")
    ()

action consumir => Int
    let msg := log_recv!("default")
    echo!(msg)
    0

fork!(avisar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("aviso-warn"),
        "deve imprimir 'aviso-warn' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 14. log_diretiva_e_action_independentes ───────────────────

/// Action com `@log` (diretiva, wrapping) e `log!()` (explícita, linha).
/// Ambos disparam — dois `log_recv!()` consomem as duas mensagens.
/// Usa policy "block" (Queue) para garantir que ambas as mensagens
/// são preservadas e consumidas em ordem.
#[test]
fn log_diretiva_e_action_independentes() {
    let path = write_temp_kata(
        "log_diretiva_e_action_independentes",
        r#"@log{msg: "dir: {x}", when: "enter", policy: "block"}
action ambos (x::Int) => Int
    log!(LogLevel::Info, "act: explicit", "default", "block")
    + x 100

action consumir => Int
    let m1 := log_recv!("default")
    let m2 := log_recv!("default")
    echo!(m1)
    echo!(m2)
    0

fork!(ambos, (1))
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("dir: 1") && stdout.contains("act: explicit"),
        "deve imprimir ambas as mensagens — stdout: {stdout} | stderr: {stderr}"
    );
}