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

pub(crate) mod ipc;
pub(crate) mod ops;
pub(crate) mod select;

// Re-exports da camada de operações — `lib.rs` continua importando os
// mesmos símbolos C-ABI.
pub use ops::{kata_rt_channel_recv, kata_rt_channel_send};
pub use select::kata_rt_select;
pub use select::kata_rt_select_combined;
// `can_recv`/`can_send` são usadas pelo scheduler (pub(crate) no ops).
/// Retorna o read_fd bruto de um handle IPC para poll unificado.
pub(crate) use ipc::ipc_read_fd;
/// Bloqueia (poll blocking) até o canal IPC ter dados. Usado pelo
/// scheduler quando todos os fibers estão blocked em IPC e não há
/// outros fibers para executar — o child OS process ainda pode escrever.
pub(crate) use ops::block_ipc_until_readable;
/// Verifica se um handle é de canal IPC (tag TAG_IPC_CHANNEL).
pub(crate) use ops::is_ipc_handle;
pub(crate) use ops::{can_recv, can_send};

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

// ── Tags nos 2 bits baixos ───────────────────────────────────────────
//
// Ponteiros de heap são 8-byte aligned → bits 0-1 são sempre 0.
// Usamos esses 2 bits para identificar a topologia.

pub(super) const TAG_CHANNEL: i64 = 0b000; // rendezvous
pub(super) const TAG_QUEUE: i64 = 0b001; // buffered
pub(super) const TAG_BROADCAST: i64 = 0b010; // sender/factory
pub(super) const TAG_BROADCAST_RX: i64 = 0b011; // receiver
pub(super) const TAG_IPC_CHANNEL: i64 = 0b100; // cross-process pipe

const TAG_MASK: i64 = 0b111;
const PTR_MASK: i64 = !0b111;

/// Extrai a tag (2 bits baixos) do handle.
pub(super) fn tag_of(handle: i64) -> i64 {
    handle & TAG_MASK
}

/// Extrai o ponteiro (bits altos) do handle, sem a tag.
pub(super) fn ptr_of(handle: i64) -> *mut u8 {
    (handle & PTR_MASK) as *mut u8
}

/// Combina ponteiro + tag num handle i64.
pub(super) fn make_handle_pub(ptr: *mut u8, tag: i64) -> i64 {
    (ptr as i64) | tag
}

fn make_handle(ptr: *mut u8, tag: i64) -> i64 {
    make_handle_pub(ptr, tag)
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

/// Política de envio quando o buffer está cheio.
///
/// `Block` — bloqueia o fiber até o consumidor liberar espaço (semântica
/// original de queue bounded). `Drop` — descarta o valor e retorna OK sem
/// bloquear (first-write-wins: o valor existente no buffer é mantido).
///
/// Usado pelo `@timer` TCO: o canal interno buffer-1 com policy Drop
/// preserva o timestamp da chamada mais externa através da destruição
/// de frames do `return_call`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Policy {
    Block,
    Drop,
}

/// Fila bufferizada — bloqueia se buffer cheio (policy Block) ou descarta
/// (policy Drop).
pub(crate) struct QueueInner {
    buffer: Mutex<VecDeque<i64>>,
    capacity: usize,
    policy: Policy,
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

/// Canal cross-process — pipe Unix com FDs de leitura e escrita.
///
/// `type_id` é o índice na type table do tipo de elemento do canal.
/// Usado por `to_bytes`/`from_bytes` para serializar/desserializar
/// valores no trânsito cross-process.
///
/// Após fork, o parent fecha `read_fd` (só envia) e o child fecha
/// `write_fd` (só recebe) — ou vice-versa, conforme o fluxo. O kernel
/// rastreia referências por FD: quando o último FD de uma ponta do
/// pipe é fechado, o pipe é destruído.
pub(crate) struct IpcChannelInner {
    pub(crate) write_fd: i32,
    pub(crate) read_fd: i32,
    pub(crate) type_id: i64,
    /// Handle do canal IPC de ack (TAG_IPC_CHANNEL). 0 = sem auto-ack
    /// (canal rendezvous simples). Quando != 0, `try_ipc_recv` envia um
    /// ack automático após desserializar — usado pelo queue!(N) IPC onde
    /// o broker precisa saber que o child consumiu o item.
    pub(crate) ack_tx_handle: i64,
}

// ── Funções auxiliares de alocação na arena ──────────────────────────

/// Aloca `size` bytes na arena e escreve `value` no espaço alocado.
/// Retorna ponteiro bruto (sem tag).
///
/// # Safety
/// `arena_handle` deve ser válido. `size` deve ser `size_of::<T>()`.
pub(super) unsafe fn arena_alloc_and_init<T>(arena_handle: i64, value: T) -> *mut u8 {
    let size = std::mem::size_of::<T>() as i64;
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena_handle, size);
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
/// `policy` controla o comportamento quando o buffer está cheio:
/// - `0` = Block (bloqueia o fiber até o consumidor liberar espaço)
/// - `1` = Drop (descarta o valor novo, mantém o existente — first-write-wins)
///
/// # Safety
/// `arena` deve ser válido. `capacity` deve ser > 0.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_queue_create(arena: i64, capacity: i64, policy: i64) -> i64 {
    if capacity <= 0 {
        return 0;
    }
    let policy = if policy == 1 {
        Policy::Drop
    } else {
        Policy::Block
    };
    let inner = QueueInner {
        buffer: Mutex::new(VecDeque::new()),
        capacity: capacity as usize,
        policy,
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

/// Cria canal cross-process via pipe Unix. Aloca `IpcChannelInner`
/// na arena do caller. Retorna handle com tag `0b100` (TAG_IPC_CHANNEL).
///
/// O `type_id` é o índice na type table do tipo de elemento do canal.
/// Usado por `to_bytes`/`from_bytes` para serializar valores.
///
/// `ack_tx_handle`: handle de outro canal IPC para auto-ack em `try_ipc_recv`.
/// 0 = sem auto-ack (rendezvous simples).
///
/// # Safety
/// `arena` deve ser válido. `type_id` deve ser um índice válido na
/// type table registrada.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_ipc_channel_create(arena: i64, type_id: i64, ack_tx_handle: i64) -> i64 {
    // SAFETY: arena é válido (contrato FFI).
    unsafe { ipc::create_ipc_channel(arena, type_id, ack_tx_handle) }
}

/// Cria queue IPC cross-process: in-process queue + IPC data channel + IPC ack channel.
///
/// Retorna ponteiro para tupla de 6 handles na arena (48 bytes):
/// `(queue_tx, queue_rx, ipc_data_tx, ipc_data_rx, ack_tx, ack_rx)`
///
/// - `queue_tx` (TAG_QUEUE): sender do usuário faz `!>` aqui
/// - `queue_rx` (TAG_QUEUE): broker drena daqui
/// - `ipc_data_tx` (TAG_IPC_CHANNEL): broker envia para o child
/// - `ipc_data_rx` (TAG_IPC_CHANNEL): child recebe (herdado via fork)
/// - `ack_tx` (TAG_IPC_CHANNEL): child envia ack (herdado via fork, auto-ack)
/// - `ack_rx` (TAG_IPC_CHANNEL): broker recebe ack
///
/// O `ipc_data_rx` tem `ack_tx_handle = ack_tx` para auto-ack em `try_ipc_recv`.
/// Os outros canais têm `ack_tx_handle = 0` (sem auto-ack).
///
/// # Safety
/// `arena` deve ser válido. `cap` deve ser > 0. `type_id` deve ser índice válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_ipc_queue_create(arena: i64, cap: i64, type_id: i64) -> i64 {
    if cap <= 0 {
        return 0;
    }

    // 1. Criar ack channel primeiro (sem auto-ack nele mesmo).
    // SAFETY: arena é válido (contrato FFI).
    let ack_rx = unsafe { ipc::create_ipc_channel(arena, 0, 0) }; // type_id=0 (Int)
    if ack_rx == 0 {
        return 0;
    }

    // 2. Criar IPC data channel com auto-ack apontando para ack_tx.
    // O ack_tx é o mesmo handle que ack_rx (mesmo pipe, direção oposta).
    // O child vai usar ack_tx (write_fd) para enviar acks; o broker usa
    // ack_rx (read_fd) para receber. O ipc_data_rx tem ack_tx_handle = ack_rx
    // (mesmo handle, pois TAG_IPC_CHANNEL é simétrico).
    // SAFETY: arena é válido.
    let ipc_data_tx = unsafe { ipc::create_ipc_channel(arena, type_id, ack_rx) };
    if ipc_data_tx == 0 {
        return 0;
    }

    // 3. Criar in-process queue.
    let queue_handle = kata_rt_queue_create(arena, cap, 0);
    if queue_handle == 0 {
        return 0;
    }

    // 4. Empacotar 6 handles na arena (48 bytes).
    // Layout: [queue_tx, queue_rx, ipc_data_tx, ipc_data_rx, ack_tx, ack_rx]
    // queue_tx = queue_rx = queue_handle (mesmo handle para in-process queue)
    // ipc_data_rx = ipc_data_tx (mesmo handle — simétrico, child usa read_fd)
    // ack_tx = ack_rx (mesmo handle — simétrico, child usa write_fd)
    let size = 48i64;
    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), arena, size);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        let p = ptr as *mut i64;
        std::ptr::write_unaligned(p, queue_handle); // queue_tx
        std::ptr::write_unaligned(p.add(1), queue_handle); // queue_rx
        std::ptr::write_unaligned(p.add(2), ipc_data_tx); // ipc_data_tx
        std::ptr::write_unaligned(p.add(3), ipc_data_tx); // ipc_data_rx (mesmo handle)
        std::ptr::write_unaligned(p.add(4), ack_rx); // ack_tx (mesmo handle)
        std::ptr::write_unaligned(p.add(5), ack_rx); // ack_rx
    }
    ptr
}
