//! Testes E2E do `@timer` — medição de tempo de execução via diretiva.
//!
//! Cada teste cria um arquivo `.kata` temporário, invoca o binário `kata`
//! via subprocess (`kata run`), e verifica stdout + exit code.
//!
//! Sintaxe de funções puras em Kata:
//! ```
//! nome :: Tipo => Tipo
//! lambda pattern: body
//! ```
//!
//! Os testes cobrem:
//!  1. timer_basico          — @timer em função pura, publica delta
//!  2. timer_com_topic       — @timer{topic: "..."} publica no tópico
//!  3. timer_msg_custom      — @timer{msg: "..."} com template custom
//!  4. now_builtin           — now!() retorna valor monotônico
//!  5. timer_tco             — @timer em função tail-recursiva (canal buffer-1)

use std::fs;
use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-timer-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

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

// ── 1. timer_basico — @timer publica delta da função ─────────

/// `@timer` em função pura: o codegen injeta `kata_rt_timer_now()` no
/// prólogo (start) e no epílogo (delta = end - start), publica via
/// `kata_rt_log_publish` no tópico default (nome da função).
/// O consumidor recebe via `log_recv!("nome-func")`.
#[test]
fn timer_basico() {
    let path = write_temp_kata(
        "timer_basico",
        r#"@timer
custo :: Int => Int
lambda n: + n 1

action chamar => Int
    let r := custo 42
    r

action consumir => Int
    let msg := log_recv!("custo")
    echo!(msg)
    0

fork!(chamar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // A mensagem default é "{name}: {delta}ns" — deve conter "custo:" e "ns".
    assert!(
        stdout.contains("custo:") && stdout.contains("ns"),
        "deve imprimir 'custo: ...ns' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 2. timer_com_topic — @timer com tópico explícito ─────────

/// `@timer{topic: "perfil"}` publica no tópico "perfil" em vez do
/// nome da função.
#[test]
fn timer_com_topic() {
    let path = write_temp_kata(
        "timer_com_topic",
        r#"@timer{topic: "perfil"}
pesada :: Int => Int
lambda n: + n 1

action chamar => Int
    let r := pesada 10
    r

action consumir => Int
    let msg := log_recv!("perfil")
    echo!(msg)
    0

fork!(chamar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("ns"),
        "deve imprimir mensagem com 'ns' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 3. timer_msg_custom — @timer com template custom ─────────

/// `@timer{msg: "{name}: demorou {delta}ns"}` usa template custom.
#[test]
fn timer_msg_custom() {
    let path = write_temp_kata(
        "timer_msg_custom",
        r#"@timer{msg: "{name}: demorou {delta}ns"}
calcula :: Int => Int
lambda n: + n 1

action chamar => Int
    let r := calcula 5
    r

action consumir => Int
    let msg := log_recv!("calcula")
    echo!(msg)
    0

fork!(chamar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("demorou") && stdout.contains("ns"),
        "deve imprimir 'demorou ...ns' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 4. now_builtin — now!() retorna valor monotônico ─────────

/// `now!()` é chamada duas vezes e o delta é computado manualmente.
/// O resultado é publicado via `log!`.
#[test]
fn now_builtin() {
    let path = write_temp_kata(
        "now_builtin",
        r#"action medir => Int
    let t0 := now!()
    let t1 := now!()
    let delta := - t1 t0
    log!(LogLevel::Info, "delta-manual", "timer-test")
    delta

action consumir => Int
    let msg := log_recv!("timer-test")
    echo!(msg)
    0

fork!(medir, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("delta-manual"),
        "deve imprimir 'delta-manual' — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── 5. timer_tco — @timer em função tail-recursiva (wrapper/inner) ──

/// `@timer` em função tail-recursiva: o wrapper/inner split preserva o
/// frame do wrapper onde `start = timer_now()` é gravado. O inner faz
/// TCO (return_call chain) e retorna para o wrapper, que mede o delta
/// completo da cadeia. O delta deve ser significativo (cadeia de 5
/// chamadas), não ~0.
#[test]
fn timer_tco() {
    let path = write_temp_kata(
        "timer_tco",
        r#"@timer
fatorial :: Int => Int
lambda 0: 1
lambda n: fatorial (- n 1)

action chamar => Int
    let r := fatorial 5
    r

action consumir => Int
    let msg := log_recv!("fatorial")
    echo!(msg)
    0

fork!(chamar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("fatorial:") && stdout.contains("ns"),
        "deve imprimir 'fatorial: ...ns' — stdout: {stdout} | stderr: {stderr}"
    );
    // O delta NÃO deve ser ~0 — o wrapper preserva o start da primeira chamada.
    // O inner faz TCO (return_call chain), retorna para o wrapper, que mede
    // delta = end - start da cadeia inteira.
    // Extraímos o número do delta da mensagem "fatorial: {delta}ns".
    let delta_str = stdout
        .lines()
        .find(|l| l.contains("fatorial:") && l.contains("ns"))
        .and_then(|l| {
            let start = l.find(':').map(|i| i + 1)?;
            let end = l.rfind("ns").filter(|&e| e > start)?;
            l[start..end].trim().parse::<i64>().ok()
        });
    if let Some(delta) = delta_str {
        // SMI-tagged: o valor real é delta >> 1. Comparamos o valor bruto.
        // Para uma cadeia de 5 chamadas, o delta deve ser > 0 (não ~0).
        // Se fosse stack slot sem wrapper, o delta seria 0 ou 1 (última chamada).
        assert!(
            delta > 1,
            "delta deve ser significativo (wrapper preserva start da primeira chamada) \
             — delta bruto: {delta}, stdout: {stdout}"
        );
    }
}

// ── 6. timer_tco_large_n — @timer mede cadeia inteira de 100k chamadas ──

/// `@timer` em `count_down 100000 0` (tail-recursivo, n grande).
/// O wrapper mede o delta da cadeia inteira (outer call → inner TCO
/// chain → base case → return → wrapper epílogue). Com n=100000, o
/// delta deve ser significativamente > 0 — prova que o wrapper mede a
/// cadeia completa, não só a última chamada.
/// Usa `count_down` (O(1) por step) em vez de `fat_tail` (computa
/// fatorial — resultado astronômico, demora infinito).
#[test]
fn timer_tco_large_n() {
    let path = write_temp_kata(
        "timer_tco_large_n",
        r#"@timer
count_down :: Int Int => Int
lambda 0 acc: acc
lambda n acc: count_down (- n 1) acc

action chamar => Int
    let r := count_down 100000 0
    r

action consumir => Int
    let msg := log_recv!("count_down")
    echo!(msg)
    0

fork!(chamar, ())
consumir!()"#,
    );

    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(
        code, 0,
        "exit 0 (TCO deve evitar stack overflow) — stderr: {stderr}"
    );
    assert!(
        stdout.contains("count_down:") && stdout.contains("ns"),
        "deve imprimir 'count_down: ...ns' — stdout: {stdout} | stderr: {stderr}"
    );
    // O delta deve ser significativo — 100k iterações, mesmo que rápidas,
    // produzem um delta mensurável. Se fosse ~0, significaria que só a
    // última chamada foi medida (bug no wrapper).
    let delta_str = stdout
        .lines()
        .find(|l| l.contains("count_down:") && l.contains("ns"))
        .and_then(|l| {
            let start = l.find(':').map(|i| i + 1)?;
            let end = l.rfind("ns").filter(|&e| e > start)?;
            l[start..end].trim().parse::<i64>().ok()
        });
    if let Some(delta) = delta_str {
        assert!(
            delta > 1,
            "delta deve ser significativo para 100k chamadas \
             — delta bruto: {delta}, stdout: {stdout}"
        );
    }
}
