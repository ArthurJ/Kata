//! Canais CSP — structs de runtime e FFI functions.
//!
//! Fase 3 do Fio 11: cria canais (rendezvous, queue, broadcast) alocados
//! na arena do fiber criador. Handles são ponteiro+tag (2 bits baixos).
//!
//! Fase 4: blocking cooperativo. Quando `send`/`recv` não pode completar e
//! há um fiber em execução (Suspend em TLS), suspende o fiber com
//! `YieldReason`. O scheduler acorda o fiber quando a operação pode prosseguir.
//!
//! **Lifetime:** canais são alocados via `kata_rt_arena_alloc` na arena
//! do fiber criador. Handles fluem apenas descendente na árvore de
//! fibers. O fiber criador é sempre o último vivo — `arena_destroy`
//! libera o canal junto. Sem tracking explícito no Scheduler.
//!
//! **Invariante:** `bumpalo::Bump` não chama destructors no drop.
//! `Mutex`/`Condvar` no Linux são futex (drop = no-op). Seguro em
//! single-threaded sem contensão. Ver PRD-fio11 §Runtime/Handle.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

// ── Tags nos 2 bits baixos ───────────────────────────────────────────
//
// Ponteiros de heap são 8-byte aligned → bits 0-1 são sempre 0.
// Usamos esses 2 bits para identificar a topologia.

const TAG_CHANNEL: i64 = 0b00; // rendezvous
const TAG_QUEUE: i64 = 0b01; // buffered
const TAG_BROADCAST: i64 = 0b10; // sender/factory
const TAG_BROADCAST_RX: i64 = 0b11; // receiver

const TAG_MASK: i64 = 0b11;
const PTR_MASK: i64 = !0b11;

/// Extrai a tag (2 bits baixos) do handle.
fn tag_of(handle: i64) -> i64 {
    handle & TAG_MASK
}

/// Extrai o ponteiro (bits altos) do handle, sem a tag.
fn ptr_of(handle: i64) -> *mut u8 {
    (handle & PTR_MASK) as *mut u8
}

/// Combina ponteiro + tag num handle i64.
fn make_handle(ptr: *mut u8, tag: i64) -> i64 {
    (ptr as i64) | tag
}

// ── Structs ──────────────────────────────────────────────────────────

/// Canal rendezvous — sender bloqueia até receptor sincronizar.
///
/// Fase 3: slot simples (sem blocking real). Fase 4: Condvar para
/// yield/bloqueio cooperativo.
pub(crate) struct ChannelInner {
    slot: Mutex<Option<i64>>,
    sender_ready: Condvar,
    receiver_ready: Condvar,
}

/// Fila bufferizada — bloqueia se buffer cheio.
pub(crate) struct QueueInner {
    buffer: Mutex<VecDeque<i64>>,
    capacity: usize,
    not_full: Condvar,
    not_empty: Condvar,
}

/// Broadcast pub-sub — fire-and-forget, latest only (Decisão F).
pub(crate) struct BroadcastInner {
    value: Mutex<Option<i64>>,
    version: Mutex<u64>,
    new_msg: Condvar,
}

/// Receiver de broadcast — um por `rxf!()`, mantém `last_seen_version`
/// próprio. Compartilha o `BroadcastInner` do sender via ponteiro.
pub(crate) struct BroadcastReceiver {
    inner: *mut BroadcastInner,
    last_seen_version: u64,
}

// ── Sentinel ─────────────────────────────────────────────────────────
//
// Retornado por send/recv quando a operação não pode completar
// (canal vazio, buffer cheio). Fase 4 substitui por blocking real.

const WOULD_BLOCK: i64 = -1;
const OK: i64 = 0;

// ── Funções auxiliares de alocação na arena ──────────────────────────

/// Aloca `size` bytes na arena e escreve `value` no espaço alocado.
/// Retorna ponteiro bruto (sem tag).
///
/// # Safety
/// `arena_handle` deve ser válido. `size` deve ser `size_of::<T>()`.
unsafe fn arena_alloc_and_init<T>(arena_handle: i64, value: T) -> *mut u8 {
    let size = std::mem::size_of::<T>() as i64;
    let ptr = crate::arena::kata_rt_arena_alloc(arena_handle, size);
    if ptr == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: ptr veio de arena_alloc (alinhado, não-null). T é o tipo
    // correto para este espaço.
    unsafe { std::ptr::write(ptr as *mut T, value) };
    ptr as *mut u8
}

// ── FFI: Criação de canais ────────────────────────────────────────────

/// Cria canal rendezvous. Aloca `ChannelInner` na arena do caller.
/// Retorna handle com tag `0b00`.
///
/// # Safety
/// `arena` deve ser um handle válido retornado por `kata_rt_arena_create`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_channel_create(arena: i64) -> i64 {
    let inner = ChannelInner {
        slot: Mutex::new(None),
        sender_ready: Condvar::new(),
        receiver_ready: Condvar::new(),
    };
    // SAFETY: arena é válido (contrato FFI). size_of::<ChannelInner> > 0.
    let ptr = unsafe { arena_alloc_and_init(arena, inner) };
    if ptr.is_null() {
        return 0;
    }
    make_handle(ptr, TAG_CHANNEL)
}

/// Cria fila bufferizada com capacidade `capacity`. Aloca `QueueInner`
/// na arena do caller. Retorna handle com tag `0b01`.
///
/// # Safety
/// `arena` deve ser válido. `capacity` deve ser > 0.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_queue_create(arena: i64, capacity: i64) -> i64 {
    if capacity <= 0 {
        return 0;
    }
    let inner = QueueInner {
        buffer: Mutex::new(VecDeque::new()),
        capacity: capacity as usize,
        not_full: Condvar::new(),
        not_empty: Condvar::new(),
    };
    // SAFETY: arena é válido (contrato FFI).
    let ptr = unsafe { arena_alloc_and_init(arena, inner) };
    if ptr.is_null() {
        return 0;
    }
    make_handle(ptr, TAG_QUEUE)
}

/// Cria broadcast. Aloca `BroadcastInner` na arena do caller.
/// Retorna handle com tag `0b10` (sender/factory).
///
/// # Safety
/// `arena` deve ser válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_broadcast_create(arena: i64) -> i64 {
    let inner = BroadcastInner {
        value: Mutex::new(None),
        version: Mutex::new(0),
        new_msg: Condvar::new(),
    };
    // SAFETY: arena é válido (contrato FFI).
    let ptr = unsafe { arena_alloc_and_init(arena, inner) };
    if ptr.is_null() {
        return 0;
    }
    make_handle(ptr, TAG_BROADCAST)
}

/// Cria um novo receiver de broadcast a partir de um handle de
/// broadcast (tag `0b10`). Aloca `BroadcastReceiver` na arena do
/// caller. Retorna handle com tag `0b11`.
///
/// O receiver é inicializado com `last_seen_version = version atual`,
/// então só vê mensagens futuras (Decisão F).
///
/// # Safety
/// `arena` deve ser válido. `factory_handle` deve ser um handle de
/// broadcast (tag `0b10`).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_broadcast_receiver_create(arena: i64, factory_handle: i64) -> i64 {
    if tag_of(factory_handle) != TAG_BROADCAST {
        return 0;
    }
    let inner_ptr = ptr_of(factory_handle) as *mut BroadcastInner;
    // SAFETY: factory_handle veio de kata_rt_broadcast_create. O ponteiro
    // é válido enquanto a arena do criador viver. O criador é always-last
    // (bottom-up destruction), então o ponteiro é válido.
    let last_seen = unsafe { *(*inner_ptr).version.lock().unwrap() };
    let rx = BroadcastReceiver {
        inner: inner_ptr,
        last_seen_version: last_seen,
    };
    // SAFETY: arena é válido (contrato FFI).
    let ptr = unsafe { arena_alloc_and_init(arena, rx) };
    if ptr.is_null() {
        return 0;
    }
    make_handle(ptr, TAG_BROADCAST_RX)
}

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
            TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                let mut slot = inner.slot.lock().unwrap();
                if slot.is_some() {
                    // Slot ocupado — receptor ainda não consumiu.
                    WOULD_BLOCK
                } else {
                    *slot = Some(value);
                    inner.receiver_ready.notify_one();
                    OK
                }
            }
            TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                let mut buffer = inner.buffer.lock().unwrap();
                if buffer.len() >= inner.capacity {
                    // Buffer cheio.
                    WOULD_BLOCK
                } else {
                    buffer.push_back(value);
                    inner.not_empty.notify_one();
                    OK
                }
            }
            TAG_BROADCAST => {
                let inner = &*(ptr as *const BroadcastInner);
                {
                    let mut val = inner.value.lock().unwrap();
                    *val = Some(value);
                }
                {
                    let mut ver = inner.version.lock().unwrap();
                    *ver += 1;
                }
                inner.new_msg.notify_all();
                OK
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
        let result = try_recv(handle);
        if result != WOULD_BLOCK {
            return result;
        }
        // Não pode receber agora. Se há fiber em execução, suspende.
        let suspended = crate::fiber::with_suspend(|suspend| {
            suspend.suspend(crate::fiber::YieldReason::WaitingOnChannel(handle));
        });
        if suspended.is_none() {
            // Fora de fiber (teste unitário) — retorna WOULD_BLOCK.
            return WOULD_BLOCK;
        }
        // Fiber foi resumido — scheduler acredita que há dado. Tentar novamente.
    }
}

/// Tenta receber sem bloquear. Retorna o valor se disponível, `WOULD_BLOCK`
/// se não há dado, `WOULD_BLOCK` para tag inválida.
///
/// Função interna extraída para permitir re-tentativa após resume.
fn try_recv(handle: i64) -> i64 {
    let tag = tag_of(handle);
    let ptr = ptr_of(handle);
    if ptr.is_null() {
        return WOULD_BLOCK;
    }
    // SAFETY: handle veio da FFI de criação correspondente.
    unsafe {
        match tag {
            TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                let mut slot = inner.slot.lock().unwrap();
                if let Some(v) = slot.take() {
                    inner.sender_ready.notify_one();
                    v
                } else {
                    WOULD_BLOCK
                }
            }
            TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                let mut buffer = inner.buffer.lock().unwrap();
                if let Some(v) = buffer.pop_front() {
                    inner.not_full.notify_one();
                    v
                } else {
                    WOULD_BLOCK
                }
            }
            TAG_BROADCAST_RX => {
                let rx = &*(ptr as *const BroadcastReceiver);
                // SAFETY: rx.inner aponta para um BroadcastInner válido
                // na arena do criador. O criador é always-last.
                let inner = &*rx.inner;
                let ver = inner.version.lock().unwrap();
                if *ver > rx.last_seen_version {
                    let val = inner.value.lock().unwrap();
                    let result = val.unwrap_or(WOULD_BLOCK);
                    // Atualiza last_seen. Preciso soltar os locks primeiro.
                    drop(val);
                    drop(ver);
                    // SAFETY: rx é &mut via ponteiro mutável (arena alloc).
                    // Como single-threaded, sem data race.
                    (*ptr.cast::<BroadcastReceiver>()).last_seen_version =
                        *inner.version.lock().unwrap();
                    result
                } else {
                    WOULD_BLOCK
                }
            }
            _ => WOULD_BLOCK, // tag inválida ou broadcast sender (não recebe)
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
            TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                inner.slot.lock().unwrap().is_some()
            }
            TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                !inner.buffer.lock().unwrap().is_empty()
            }
            TAG_BROADCAST_RX => {
                let rx = &*(ptr as *const BroadcastReceiver);
                let inner = &*rx.inner;
                *inner.version.lock().unwrap() > rx.last_seen_version
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
            TAG_CHANNEL => {
                let inner = &*(ptr as *const ChannelInner);
                inner.slot.lock().unwrap().is_none()
            }
            TAG_QUEUE => {
                let inner = &*(ptr as *const QueueInner);
                inner.buffer.lock().unwrap().len() < inner.capacity
            }
            TAG_BROADCAST => true, // broadcast sempre pode enviar
            _ => false,
        }
    }
}

// ── FFI: Select ──────────────────────────────────────────────────────

/// Tenta receber de qualquer receiver na lista. Retorna
/// `(índice, valor)` empacotado como i64 (high 32 = índice, low 32 =
/// valor). Se nenhum canal tem dado, retorna `WOULD_BLOCK` (-1).
///
/// Fase 3: versão simples sem timeout (timeout vem na Fase 6).
/// Não bloqueia — se nenhum canal tem dado, retorna imediatamente.
///
/// # Safety
/// `handles` deve apontar para um array de `n_handles` handles
/// válidos de canal (receiver side).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn kata_rt_select(handles: *const i64, n_handles: i64) -> i64 {
    if handles.is_null() || n_handles <= 0 {
        return WOULD_BLOCK;
    }
    // SAFETY: handles é um ponteiro válido para n_handles i64s (contrato FFI).
    let handles_slice = unsafe { std::slice::from_raw_parts(handles, n_handles as usize) };
    for (idx, &handle) in handles_slice.iter().enumerate() {
        let val = try_recv(handle);
        if val != WOULD_BLOCK {
            // Pack: high 32 = idx, low 32 = value.
            // Valores SMI cabem em 32 bits (inteiros pequenos).
            return ((idx as i64) << 32) | (val & 0xFFFF_FFFF);
        }
    }
    WOULD_BLOCK
}
