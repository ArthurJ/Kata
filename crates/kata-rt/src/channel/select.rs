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

// TLS: offset de rodízio do select combinado. Persiste entre chamadas
// para que braços always-ready (ex: arquivo regular) não façam starvation
// dos outros braços. Resetado em `reset_tls_between_runs`.
thread_local! {
    pub(crate) static SELECT_ROTATION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Reseta o rodízio do select (chamado por `reset_tls_between_runs`).
pub(crate) fn reset_select_rotation() {
    SELECT_ROTATION.with(|r| r.set(0));
}

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
            suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                channel_handles: handles_vec.clone(),
                file_handles: Vec::new(),
                socket_handles: Vec::new(),
                deadline,
            });
        });
        if suspended.is_none() {
            // Fora de fiber (teste unitário) — retorna WOULD_BLOCK.
            return WOULD_BLOCK;
        }
        // Fiber resumido — scheduler acredita que há dado ou timeout expirou.
        // Loop tenta novamente.
    }
}

// Sentinel de timeout do select. Rebaixado de `pub` para `pub(crate)` na
/// Select combinado — multiplexa channels e file handles numa única chamada.
///
/// Retorna índice global:
/// - `0..n_c-1`: channel arm `j` pronto (j = índice relativo dentro de channels)
/// - `n_c..n_c+n_f-1`: file arm `j` pronto (índice global = n_c + j)
/// - `WOULD_BLOCK` (-1): nenhum handle pronto (fora de fiber)
/// - `SELECT_TIMEOUT` (-2): timeout expirou
///
/// Tenta channels (`can_recv`) e files (`poll(POLLIN, 0)`) num único loop.
/// Se nenhum pronto, suspende o fiber **uma vez** com `WaitingOnSelect`
/// carregando ambos os conjuntos de handles. Após resume, tenta novamente.
///
/// Isto resolve o gatekeeper de suspensão combinada: o fiber suspende
/// esperando AMBOS os conjuntos simultaneamente, em vez de duas FFIs
/// que cada uma suspende independentemente.
///
/// # Safety
/// `chan_handles` deve apontar para `n_c` handles de canal válidos (ou ser
/// null se `n_c <= 0`). `file_handles` deve apontar para `n_f` handles de
/// File válidos (ou ser null se `n_f <= 0`).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn kata_rt_select_combined(
    chan_handles: *const i64,
    n_c: i64,
    file_handles: *const i64,
    n_f: i64,
    socket_handles: *const i64,
    n_s: i64,
    timeout_ms: i64,
) -> i64 {
    let chan_slice: &[i64] = if !chan_handles.is_null() && n_c > 0 {
        // SAFETY: contrato FFI — chan_handles aponta para n_c i64s válidos.
        unsafe { std::slice::from_raw_parts(chan_handles, n_c as usize) }
    } else {
        &[]
    };
    let file_slice: &[i64] = if !file_handles.is_null() && n_f > 0 {
        // SAFETY: contrato FFI — file_handles aponta para n_f i64s válidos.
        unsafe { std::slice::from_raw_parts(file_handles, n_f as usize) }
    } else {
        &[]
    };
    let socket_slice: &[i64] = if !socket_handles.is_null() && n_s > 0 {
        // SAFETY: contrato FFI — socket_handles aponta para n_s i64s válidos.
        unsafe { std::slice::from_raw_parts(socket_handles, n_s as usize) }
    } else {
        &[]
    };

    if chan_slice.is_empty() && file_slice.is_empty() && socket_slice.is_empty() {
        return WOULD_BLOCK;
    }

    let chan_vec: Vec<i64> = chan_slice.to_vec();
    let file_vec: Vec<i64> = file_slice.to_vec();
    let socket_vec: Vec<i64> = socket_slice.to_vec();

    // Total de braços no select — usado para rodízio.
    let total_arms = (n_c + n_f + n_s) as usize;

    let deadline = if timeout_ms > 0 {
        Some(std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64))
    } else {
        None
    };

    // Rodízio: a cada chamada do select, começar a checagem a partir do
    // próximo braço. Isto previne starvation quando um braço always-ready
    // (ex: arquivo regular) sempre vence. O offset persiste entre chamadas
    // via TLS — cada chamada ao select começa onde a anterior parou.
    let rotation = SELECT_ROTATION.with(|r| r.get());

    loop {
        // 1. Percorrer braços na ordem rotacionada.
        //    Para cada offset i no range [rotation, rotation+total_arms),
        //    checar o braço correspondente.
        let mut found: i64 = WOULD_BLOCK;
        for step in 0..total_arms {
            let global_idx = (rotation + step) % total_arms;
            if global_idx < n_c as usize {
                // Channel arm.
                if can_recv(chan_slice[global_idx]) {
                    found = global_idx as i64;
                    break;
                }
            } else if global_idx < (n_c + n_f) as usize {
                // File arm.
                let file_idx = global_idx - n_c as usize;
                // Poll non-blocking no file handle.
                let one_handle = [file_slice[file_idx]];
                if crate::file::try_select_files(&one_handle) != crate::file::FILE_WOULD_BLOCK {
                    found = global_idx as i64;
                    break;
                }
            } else {
                // Socket arm.
                let sock_idx = global_idx - (n_c + n_f) as usize;
                let one_handle = [socket_slice[sock_idx]];
                if crate::socket::try_select_sockets(&one_handle)
                    != crate::socket::SOCKET_WOULD_BLOCK
                {
                    found = global_idx as i64;
                    break;
                }
            }
        }

        if found != WOULD_BLOCK {
            // Avança rodízio para a próxima chamada começar no braço seguinte.
            SELECT_ROTATION.with(|r| r.set((rotation + 1) % total_arms));
            return found;
        }

        // 2. Verificar timeout.
        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
        {
            return SELECT_TIMEOUT;
        }

        // 3. Suspender com todos os conjuntos de handles.
        let suspended = crate::fiber::with_suspend(|suspend| {
            suspend.suspend(crate::fiber::YieldReason::WaitingOnSelect {
                channel_handles: chan_vec.clone(),
                file_handles: file_vec.clone(),
                socket_handles: socket_vec.clone(),
                deadline,
            });
        });
        if suspended.is_none() {
            // Fora de fiber (teste unitário) — retorna WOULD_BLOCK.
            return WOULD_BLOCK;
        }
        // Fiber resumido — scheduler acorda quando algum handle ficou pronto
        // ou o deadline expirou. Loop tenta novamente com rodízio atualizado.
    }
}
