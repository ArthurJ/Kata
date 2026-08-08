//! Timer — clock monotônico para medição de tempo.
//!
//! `kata_rt_timer_now()` retorna nanossegundos desde a primeira chamada.
//! Usa `Instant::elapsed()` a partir de um epoch preguiçoso para garantir
//! monoticidade sem depender de epoch absoluto (que pode ser não-mono-
//! tônico em algumas plataformas). `OnceLock` é thread-safe — seguro
//! para o plano de multithreading.
//!
//! Consumido por:
//! - `@timer` diretiva (codegen injeta no prólogo/epílogo)
//! - `now!()` action builtin (chamada explícita do usuário)

use std::sync::OnceLock;
use std::time::Instant;

/// Epoch do clock monotônico — inicializado uma vez, thread-safe.
static TIMER_EPOCH: OnceLock<Instant> = OnceLock::new();

/// `kata_rt_timer_now() -> i64` — clock monotônico em nanossegundos.
///
/// Retorna nanossegundos desde a primeira chamada, SMI-tagged
/// (compatível com `Int` de Kata: `(val << 1) | 1`).
/// O epoch é inicializado preguiçosamente na primeira invocação e
/// mantido em `OnceLock` (thread-safe).
///
/// Nanossegundos desde epoch cabe em SMI (62 bits ≈ 146 anos).
///
/// Monoticidade: `Instant` garante monotonicidade no Linux/Windows/macOS.
/// Não precisa de reset entre testes — o epoch é global e stateless
/// (apenas leitura do clock do SO).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_timer_now() -> i64 {
    let epoch = TIMER_EPOCH.get_or_init(Instant::now);
    let nanos = epoch.elapsed().as_nanos() as i64;
    // SMI tag: (val << 1) | 1 — mesmo esquema de kata_rt_list_len.
    (nanos << 1) | 1
}
