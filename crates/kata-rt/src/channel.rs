//! Canais CSP — structs de runtime e FFI de criação.
//!
//! Cria canais (rendezvous, queue, broadcast) alocados
//! na arena do fiber criador. Handles são ponteiro+tag (2 bits baixos).
//!
//! Blocking cooperativo. Quando `send`/`recv` não pode completar e
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
//!
//! **Estrutura do móduto:** este arquivo (módulo pai) concentra as
//! structs de canal, as tags de handle, os helpers de
//! tag/ptr/make_handle, a FFI de **criação** de canais e o helper de
//! alocação na arena. As operações de send/recv/can_recv/can_send
//! estão em [`ops`] e a FFI de select em [`select`].

pub(crate) mod ops;
pub(crate) mod select;

// Re-exports da camada de operações — `lib.rs` continua importando os
// mesmos símbolos C-ABI.
pub use ops::{kata_rt_channel_recv, kata_rt_channel_send};
pub use select::kata_rt_select;
// `can_recv`/`can_send` são usadas pelo scheduler (pub(crate) no ops).
pub(crate) use ops::{can_recv, can_send};

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

// ── Tags nos 2 bits baixos ───────────────────────────────────────────
//
// Ponteiros de heap são 8-byte aligned → bits 0-1 são sempre 0.
// Usamos esses 2 bits para identificar a topologia.

pub(super) const TAG_CHANNEL: i64 = 0b00; // rendezvous
pub(super) const TAG_QUEUE: i64 = 0b01; // buffered
pub(super) const TAG_BROADCAST: i64 = 0b10; // sender/factory
pub(super) const TAG_BROADCAST_RX: i64 = 0b11; // receiver

const TAG_MASK: i64 = 0b11;
const PTR_MASK: i64 = !0b11;

/// Extrai a tag (2 bits baixos) do handle.
pub(super) fn tag_of(handle: i64) -> i64 {
    handle & TAG_MASK
}

/// Extrai o ponteiro (bits altos) do handle, sem a tag.
pub(super) fn ptr_of(handle: i64) -> *mut u8 {
    (handle & PTR_MASK) as *mut u8
}

/// Combina ponteiro + tag num handle i64.
fn make_handle(ptr: *mut u8, tag: i64) -> i64 {
    (ptr as i64) | tag
}

// ── Structs ──────────────────────────────────────────────────────────

/// Canal rendezvous — sender bloqueia até receptor sincronizar.
///
/// Slot simples (sem blocking real). A versão com yield usa Condvar para
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
    let last_seen = unsafe {
        *(*inner_ptr)
            .version
            .lock()
            .expect("version mutex não envenenado")
    };
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
