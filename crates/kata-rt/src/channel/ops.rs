//! Operações de canal — send, recv, e verificação sem consumo
//! (`can_recv`/`can_send`) usada pelo wake pass do scheduler.
//!
//! Este submódulo concentra a lógica de despacho por tag para operações
//! que consomem ou inspecionam o estado interno dos canais. As structs de
//! canal (`ChannelInner`, `QueueInner`, `BroadcastInner`, `BroadcastReceiver`)
//! e a FFI de criação permanecem no módulo pai [`crate::channel`].
//!
//! **Tags despachadas:**
//! - `TAG_CHANNEL` (0b00) — rendezvous (slot único).
//! - `TAG_QUEUE` (0b01) — fila bufferizada.
//! - `TAG_BROADCAST` (0b10) — pub/sub fire-and-forget (sender/factory).
//! - `TAG_BROADCAST_RX` (0b11) — receiver de broadcast.

use super::{
    BroadcastInner, BroadcastReceiver, ChannelInner, Policy, QueueInner, TAG_IPC_CHANNEL, ptr_of,
    tag_of,
};

/// Verifica se um handle é de canal IPC (tag TAG_IPC_CHANNEL).
pub(crate) fn is_ipc_handle(handle: i64) -> bool {
    tag_of(handle) == TAG_IPC_CHANNEL
}

/// Bloqueia (poll blocking) até o canal IPC ter dados legíveis.
/// Usado pelo scheduler quando todos os fibers estão blocked em IPC.
pub(crate) unsafe fn block_ipc_until_readable(handle: i64) {
    // SAFETY: handle veio de um fiber blocked em canal IPC.
    unsafe { super::ipc::block_until_readable(handle) }
}

// ── Sentinel ─────────────────────────────────────────────────────────
//
// Retornado por send/recv quando a operação não pode completar
// (canal vazio, buffer cheio). A versão com yield substitui por blocking real.

pub(super) const WOULD_BLOCK: i64 = -1;
pub(super) const OK: i64 = 0;

// ── FFI: Envio (operador !>) ──────────────────────────────────────────

/// Envia valor por um handle de canal. Despacha pela tag nos 2 bits
/// baixos.
///
/// Retorna `0` (OK) se a operação completou. Se a operação bloquearia
/// (canal rendezvous com slot ocupado, queue cheia):
/// - **Dentro de um fiber** (Suspend em TLS): suspende o fiber com
///   `YieldReason::WaitingOnChannelSend(handle)`. O scheduler acorda o
///   fiber quando há espaço. Quando resumido, tenta novamente (loop).
/// - **Fora de um fiber** (teste unitário): retorna `WOULD_BLOCK` (-1).
///
/// # Safety
/// `handle` deve ser um handle válido de Channel, Queue, ou Broadcast.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_channel_send(handle: i64, value: i64) -> i64 {
    loop {
        let result = try_send(handle, value);
        if result != WOULD_BLOCK {
            return result;
        }
        // Não pode enviar agora. Se há fiber em execução, suspende.
        let suspended = crate::fiber::with_suspend(|suspend| {
            suspend.suspend(crate::fiber::YieldReason::WaitingOnChannelSend(handle));
        });
        if suspended.is_none() {
            // Fora de fiber (teste unitário) — retorna WOULD_BLOCK.
            return WOULD_BLOCK;
        }
        // Fiber foi resumido — scheduler acredita que pode enviar. Tentar novamente.
    }
}

/// Tenta enviar sem bloquear. Retorna `OK` se completou, `WOULD_BLOCK` se
/// não pode, `WOULD_BLOCK` para tag inválida.
///
/// Função interna extraída para permitir re-tentativa após resume.
fn try_send(handle: i64, value: i64) -> i64 {
    let tag = tag_of(handle);
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return WOULD_BLOCK;
    }
    // SAFETY: handle veio da FFI de criação correspondente. O ponteiro
    // é válido na arena. Tag identifica o tipo correto.
    unsafe {
        match tag {
            super::TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                let mut slot = inner
                    .slot
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime");
                if slot.is_some() {
                    // Slot ocupado — receptor ainda não consumiu.
                    WOULD_BLOCK
                } else {
                    *slot = Some(value);
                    inner.receiver_ready.notify_one();
                    OK
                }
            }
            super::TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                let mut buffer = inner
                    .buffer
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime");
                if buffer.len() >= inner.capacity {
                    // Buffer cheio — despacha por policy.
                    match inner.policy {
                        Policy::Block => WOULD_BLOCK,
                        Policy::Drop => {
                            // First-write-wins: descarta o valor novo,
                            // mantém o existente. Não bloqueia.
                            OK
                        }
                    }
                } else {
                    buffer.push_back(value);
                    inner.not_empty.notify_one();
                    OK
                }
            }
            super::TAG_BROADCAST => {
                let inner = &*(ptr as *const BroadcastInner);
                {
                    let mut val = inner
                        .value
                        .lock()
                        .expect("mutex never poisoned: single-threaded cooperative runtime");
                    *val = Some(value);
                }
                {
                    let mut ver = inner
                        .version
                        .lock()
                        .expect("mutex never poisoned: single-threaded cooperative runtime");
                    *ver += 1;
                }
                inner.new_msg.notify_all();
                OK
            }
            TAG_IPC_CHANNEL => {
                // SAFETY: handle veio de kata_rt_ipc_channel_create.
                super::ipc::try_ipc_send(handle, value)
            }
            _ => WOULD_BLOCK, // tag inválida ou broadcast receiver (não envia)
        }
    }
}

// ── FFI: Recebimento (operador <!) ────────────────────────────────────

/// Recebe valor por um handle de canal. Despacha pela tag.
///
/// Retorna o valor se disponível. Se a operação bloquearia (canal vazio):
/// - **Dentro de um fiber** (Suspend em TLS): suspende o fiber com
///   `YieldReason::WaitingOnChannel(handle)`. O scheduler acorda o
///   fiber quando há dado. Quando resumido, tenta novamente (loop).
/// - **Fora de um fiber** (teste unitário): retorna `WOULD_BLOCK` (-1).
///
/// # Safety
/// `handle` deve ser um handle válido de Channel, Queue, ou
/// BroadcastReceiver.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_channel_recv(handle: i64) -> i64 {
    loop {
        let mut out: i64 = 0;
        let has_value = try_recv(handle, &mut out);
        if has_value {
            return out; // valor de usuário — pode ser qualquer i64
        }
        // Não há dado. Se há fiber em execução, suspende.
        let suspended = crate::fiber::with_suspend(|suspend| {
            suspend.suspend(crate::fiber::YieldReason::WaitingOnChannel(handle));
        });
        if suspended.is_none() {
            // Fora de fiber — pode ser o child após spawn! (processo OS sem
            // scheduler). Se for canal IPC, bloqueia até dados chegarem via
            // poll blocking. Se não for IPC, retorna WOULD_BLOCK.
            if tag_of(handle) == TAG_IPC_CHANNEL {
                unsafe {
                    super::ipc::block_until_readable(handle);
                }
                continue;
            }
            return WOULD_BLOCK;
        }
        // Fiber foi resumido — scheduler acredita que há dado. Tentar novamente.
    }
}

/// Tenta receber sem bloquear. Retorna `true` se há valor (escrito em
/// `out`), `false` se não há dado ou tag inválida.
///
/// Usa out-parameter em vez de sentinel para evitar colisão entre
/// valores de usuário (e.g. `-1`) e o sinal de "canal vazio".
///
/// Função interna extraída para permitir re-tentativa após resume.
fn try_recv(handle: i64, out: *mut i64) -> bool {
    let tag = tag_of(handle);
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return false;
    }
    // SAFETY: handle veio da FFI de criação correspondente.
    unsafe {
        match tag {
            super::TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                let mut slot = inner
                    .slot
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime");
                if let Some(v) = slot.take() {
                    inner.sender_ready.notify_one();
                    *out = v;
                    true
                } else {
                    false
                }
            }
            super::TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                let mut buffer = inner
                    .buffer
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime");
                if let Some(v) = buffer.pop_front() {
                    inner.not_full.notify_one();
                    *out = v;
                    true
                } else {
                    false
                }
            }
            super::TAG_BROADCAST_RX => {
                let rx = &*(ptr as *const BroadcastReceiver);
                // SAFETY: rx.inner aponta para um BroadcastInner válido
                // na arena do criador. O criador é always-last.
                let inner = &*rx.inner;
                let ver = inner
                    .version
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime");
                if *ver > rx.last_seen_version {
                    let val = inner
                        .value
                        .lock()
                        .expect("mutex never poisoned: single-threaded cooperative runtime");
                    *out = val.unwrap_or(0);
                    // Atualiza last_seen. Preciso soltar os locks primeiro.
                    drop(val);
                    drop(ver);
                    // SAFETY: rx é &mut via ponteiro mutável (arena alloc).
                    // Como single-threaded, sem data race.
                    (*ptr.cast::<BroadcastReceiver>()).last_seen_version = *inner
                        .version
                        .lock()
                        .expect("mutex never poisoned: single-threaded cooperative runtime");
                    true
                } else {
                    false
                }
            }
            TAG_IPC_CHANNEL => {
                // Usa a root_arena (TLS) para alocar o valor desserializado.
                let arena = crate::arena::kata_rt_get_root_arena_handle();
                // SAFETY: handle veio de kata_rt_ipc_channel_create.
                super::ipc::try_ipc_recv(handle, arena, out)
            }
            _ => false, // tag inválida ou broadcast sender (não recebe)
        }
    }
}

// ── Verificação sem consumo (para wake pass do scheduler) ────────────

/// Verifica se `kata_rt_channel_recv(handle)` retornaria um valor (não
/// WOULD_BLOCK) **sem consumir o valor**. Usado pelo scheduler no wake
/// pass para decidir se um fiber blocked em recv pode ser acordado.
///
/// # Safety
/// `handle` deve ser um handle válido.
pub(crate) fn can_recv(handle: i64) -> bool {
    let tag = tag_of(handle);
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return false;
    }
    // SAFETY: handle veio da FFI de criação correspondente.
    unsafe {
        match tag {
            super::TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                inner
                    .slot
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime")
                    .is_some()
            }
            super::TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                !inner
                    .buffer
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime")
                    .is_empty()
            }
            super::TAG_BROADCAST_RX => {
                let rx = &*(ptr as *const BroadcastReceiver);
                let inner = &*rx.inner;
                *inner
                    .version
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime")
                    > rx.last_seen_version
            }
            TAG_IPC_CHANNEL => {
                // SAFETY: handle veio de kata_rt_ipc_channel_create.
                super::ipc::can_ipc_recv(handle)
            }
            _ => false,
        }
    }
}

/// Verifica se `kata_rt_channel_send(handle, _)` retornaria OK (não
/// WOULD_BLOCK) **sem enviar**. Usado pelo scheduler no wake pass para
/// decidir se um fiber blocked em send pode ser acordado.
///
/// Broadcast sempre pode enviar (fire-and-forget), então `can_send`
/// retorna `true` para broadcast.
///
/// # Safety
/// `handle` deve ser um handle válido.
pub(crate) fn can_send(handle: i64) -> bool {
    let tag = tag_of(handle);
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return false;
    }
    // SAFETY: handle veio da FFI de criação correspondente.
    unsafe {
        match tag {
            super::TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                inner
                    .slot
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime")
                    .is_none()
            }
            super::TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                inner
                    .buffer
                    .lock()
                    .expect("mutex never poisoned: single-threaded cooperative runtime")
                    .len()
                    < inner.capacity
            }
            super::TAG_BROADCAST => true, // broadcast sempre pode enviar
            TAG_IPC_CHANNEL => {
                // SAFETY: handle veio de kata_rt_ipc_channel_create.
                super::ipc::can_ipc_send(handle)
            }
            _ => false,
        }
    }
}
