//! Hardening do runtime — boundary FFI de BigInt contra null.
//!
//! Responsabilidade: cravar que `deref_bigint(0)` (null — slot
//! não-inicializado que escapou do typeck) NUNCA produz SIGSEGV
//! silencioso. Contrato: abort COM mensagem de diagnóstico na stderr
//! ("bug do compilador"), sinal SIGABRT — nunca signal 11 (SIGSEGV).
//!
//! O crash original (2026-08-29): pattern-binding lido fora do braço de
//! match passava slot 0 para `kata_rt_bi_show` → deref cego → SIGSEGV.
//! A via compile-time fechou com o escopo plano (`type.unbound_name`);
//! este teste protege o boundary do runtime (defense-in-depth).
//!
//! Panic em `extern "C"` não pode unwind → aborta o processo. Por isso
//! o oráculo roda em SUBPROCESSO: o filho chama o boundary com null e
//! aborta; o pai verifica mensagem + sinal.

use kata_rt::kata_rt_bi_show;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;

#[test]
fn bi_show_nunca_sigsegv_silencioso() {
    // Modo filho: chama o boundary com null — deve abortar com mensagem.
    if std::env::var("K5_NULL_DEREF_CHILD").is_ok() {
        let p = kata_rt_bi_show(0);
        let _ = unsafe { std::ffi::CStr::from_ptr(p) };
        // Se chegou aqui, retornou valor SEM abortar — contrato quebrado
        // (fallback silencioso). Filho sai 0 = pai falha.
        std::process::exit(0);
    }

    // Modo pai: re-executa este próprio teste em subprocesso.
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .args(["bi_show_nunca_sigsegv_silencioso", "--exact", "--nocapture"])
        .env("K5_NULL_DEREF_CHILD", "1")
        .output()
        .expect("subprocesso do teste deve rodar");

    let stderr = String::from_utf8_lossy(&out.stderr);

    // Contrato 1: mensagem de diagnóstico presente (erro claro, não mudo).
    assert!(
        stderr.contains("deref de valor null") && stderr.contains("bug do compilador"),
        "abort sem mensagem de diagnóstico — stderr: {stderr}"
    );

    // Contrato 2: abort (SIGABRT, sinal 6), NUNCA SIGSEGV bruto (sinal 11).
    let signal = out.status.signal();
    assert_eq!(
        signal,
        Some(6),
        "esperado SIGABRT com mensagem; obtive signal {signal:?} — SIGSEGV bruto é proibido"
    );

    // Contrato 3: o filho não pode ter "sobrevivido" (exit 0 = fallback).
    assert!(
        out.status.code() != Some(0),
        "boundary retornou valor para null — fallback silencioso viola erro-claro"
    );
}

#[test]
fn bi_show_smi_continua_funcionando() {
    // SMI (LSB=1) é o caminho legal — não pode regredir.
    // encode_smi(5) = (5 << 1) | 1 = 11
    let p = kata_rt_bi_show(11);
    let s = unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .to_string();
    assert_eq!(s, "5");
}
#[test]
fn bi_add_nunca_sigsegv_silencioso() {
    // Mesmo contrato, via aritmética (kata_rt_bi_add com null).
    if std::env::var("K5_NULL_DEREF_CHILD_ADD").is_ok() {
        let _ = kata_rt::kata_rt_bi_add(0, 11);
        std::process::exit(0);
    }
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .args(["bi_add_nunca_sigsegv_silencioso", "--exact", "--nocapture"])
        .env("K5_NULL_DEREF_CHILD_ADD", "1")
        .output()
        .expect("subprocesso deve rodar");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("deref de valor null"),
        "abort sem mensagem — stderr: {stderr}"
    );
    assert_eq!(
        out.status.signal(),
        Some(6),
        "esperado SIGABRT com mensagem; obtive {:?}",
        out.status.signal()
    );
    assert!(out.status.code() != Some(0));
}
