//! Select multiplexado — recebe de qualquer canal na lista.
//!
//! Submódulo de [`crate::channel`] com a FFI `kata_rt_select` e a
//! função interna `try_select`. As structs de canal, tags e o sentinel
//! `WOULD_BLOCK` vivem no módulo pai / em [`super::ops`].
//!
//! **Blocking cooperativo:** se nenhum canal tem dado e há um fiber em
//! execução (Suspend em TLS), suspende o fiber com
//! `YieldReason::WaitingOnSelect(handles, deadline)`. O scheduler acorda
//! o fiber quando algum canal tem dado ou o deadline expira.

use super::ops::{WOULD_BLOCK, can_recv};

/// Sentinel: timeout expirado (distinto de WOULD_BLOCK).
pub(super) const SELECT_TIMEOUT: i64 = -2;

/// Tenta encontrar um canal pronto para recebimento (sem consumir).
/// Retorna o índice (0..N-1) se algum canal tem dado, ou `WOULD_BLOCK` se
/// nenhum tem. Usa `can_recv` (não-consome) — o codegen faz `channel_recv`
/// no canal selecionado depois.
///
/// Função interna extraída para permitir re-tentativa após resume.
fn try_select(handles_slice: &[i64]) -> i64 {
    for (idx, &handle) in handles_slice.iter().enumerate() {
        if can_recv(handle) {
            return idx as i64;
        }
    }
    WOULD_BLOCK
}

/// Select multiplexado — recebe de qualquer canal na lista.
///
/// Retorna o índice (0..N-1) do primeiro canal com dado disponível.
/// O codegen então chama `kata_rt_channel_recv(handles[idx])` para consumir
/// o valor. Single-threaded cooperativo: nenhum outro fiber executa entre
/// `select_idx` e `recv`, então não há race condition.
///
/// - `timeout_ms`: se > 0, dispara timeout após N ms. Se <= 0, espera
///   indefinidamente.
/// - Retorna `SELECT_TIMEOUT` (-2) se o timeout expirou.
/// - Retorna `WOULD_BLOCK` (-1) se chamado fora de fiber e nenhum canal
///   tem dado.
///
/// **Blocking cooperativo:** se nenhum canal tem dado e há um fiber em
/// execução (Suspend em TLS), suspende o fiber com
/// `YieldReason::WaitingOnSelect(handles, deadline)`. O scheduler acorda
/// o fiber quando algum canal tem dado ou o deadline expira.
///
/// # Safety
/// `handles` deve apontar para um array de `n_handles` handles válidos
/// de canal (receiver side).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn kata_rt_select(handles: *const i64, n_handles: i64, timeout_ms: i64) -> i64 {
    if handles.is_null() || n_handles <= 0 {
        return WOULD_BLOCK;
    }
    // SAFETY: handles é um ponteiro válido para n_handles i64s (contrato FFI).
    let handles_slice = unsafe { std::slice::from_raw_parts(handles, n_handles as usize) };
    let handles_vec: Vec<i64> = handles_slice.to_vec();

    // Calcular deadline se timeout_ms > 0.
    let deadline = if timeout_ms > 0 {
        Some(std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64))
    } else {
        None
    };

    loop {
        // 1. Tentar todos os canais (sem consumir).
        let result = try_select(handles_slice);
        if result != WOULD_BLOCK {
            return result;
        }

        // 2. Verificar timeout.
        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
        {
            return SELECT_TIMEOUT;
        }

        // 3. Suspende o fiber com WaitingOnSelect.
        let suspended = crate::fiber::with_suspend(|suspend| {
            suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect(
                handles_vec.clone(),
                deadline,
            ));
        });
        if suspended.is_none() {
            // Fora de fiber (teste unitário) — retorna WOULD_BLOCK.
            return WOULD_BLOCK;
        }
        // Fiber resumido — scheduler acredita que há dado ou timeout expirou.
        // Loop tenta novamente.
    }
}

#[allow(dead_code)] // Exportado para testes externos verificarem o sentinel
pub const SELECT_TIMEOUT_SENTINEL: i64 = SELECT_TIMEOUT;
