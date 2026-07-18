//! Testes unitários do runtime de canais CSP (Fio 11 Fase 3).
//!
//! Sem JIT — testa as FFI functions diretamente. Cada teste cria uma
//! arena via `kata_rt_arena_create`, passa para as funções de criação
//! de canal, e verifica comportamento.
//!
//! Fase 3: sem blocking real. `WOULD_BLOCK` (-1) é retornado quando
//! a operação não pode completar. Fase 4 substitui por yield.

use kata_rt::{
    kata_rt_arena_create, kata_rt_arena_destroy, kata_rt_broadcast_create,
    kata_rt_broadcast_receiver_create, kata_rt_channel_create, kata_rt_channel_recv,
    kata_rt_channel_send, kata_rt_queue_create, kata_rt_select,
};

const WOULD_BLOCK: i64 = -1;
const OK: i64 = 0;

// ── Channel (rendezvous) ──────────────────────────────────────────

#[test]
fn channel_create_returns_valid_handle() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_channel_create(arena);
    assert_ne!(handle, 0, "handle não deve ser 0 (null)");
    // Tag 0b00 (channel) nos 2 bits baixos.
    assert_eq!(handle & 0b11, 0b00, "tag deve ser 0b00 (channel)");
    kata_rt_arena_destroy(arena);
}

#[test]
fn channel_send_then_recv_succeeds() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_channel_create(arena);
    let status = kata_rt_channel_send(handle, 42);
    assert_eq!(status, OK, "send em canal vazio deve retornar OK");
    let val = kata_rt_channel_recv(handle);
    assert_eq!(val, 42, "recv deve retornar o valor enviado");
    kata_rt_arena_destroy(arena);
}

#[test]
fn channel_recv_empty_returns_would_block() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_channel_create(arena);
    let val = kata_rt_channel_recv(handle);
    assert_eq!(
        val, WOULD_BLOCK,
        "recv em canal vazio deve retornar WOULD_BLOCK"
    );
    kata_rt_arena_destroy(arena);
}

#[test]
fn channel_send_full_returns_would_block() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_channel_create(arena);
    let status1 = kata_rt_channel_send(handle, 10);
    assert_eq!(status1, OK, "primeiro send deve OK");
    let status2 = kata_rt_channel_send(handle, 20);
    assert_eq!(
        status2, WOULD_BLOCK,
        "segundo send (slot ocupado) deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(arena);
}

// ── Queue (buffered) ─────────────────────────────────────────────

#[test]
fn queue_create_returns_valid_handle() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_queue_create(arena, 3);
    assert_ne!(handle, 0, "handle não deve ser 0");
    assert_eq!(handle & 0b11, 0b01, "tag deve ser 0b01 (queue)");
    kata_rt_arena_destroy(arena);
}

#[test]
fn queue_send_recv_buffered() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_queue_create(arena, 3);
    // Múltiplos sends cabem no buffer sem bloquear.
    assert_eq!(kata_rt_channel_send(handle, 1), OK);
    assert_eq!(kata_rt_channel_send(handle, 2), OK);
    assert_eq!(kata_rt_channel_send(handle, 3), OK);
    // FIFO.
    assert_eq!(kata_rt_channel_recv(handle), 1);
    assert_eq!(kata_rt_channel_recv(handle), 2);
    assert_eq!(kata_rt_channel_recv(handle), 3);
    kata_rt_arena_destroy(arena);
}

#[test]
fn queue_full_returns_would_block() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_queue_create(arena, 2);
    assert_eq!(kata_rt_channel_send(handle, 1), OK);
    assert_eq!(kata_rt_channel_send(handle, 2), OK);
    assert_eq!(
        kata_rt_channel_send(handle, 3),
        WOULD_BLOCK,
        "send com buffer cheio deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(arena);
}

#[test]
fn queue_empty_returns_would_block() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_queue_create(arena, 2);
    assert_eq!(
        kata_rt_channel_recv(handle),
        WOULD_BLOCK,
        "recv em queue vazia deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(arena);
}

#[test]
fn queue_recv_makes_space_for_send() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_queue_create(arena, 1);
    assert_eq!(kata_rt_channel_send(handle, 1), OK);
    assert_eq!(kata_rt_channel_send(handle, 2), WOULD_BLOCK);
    // Após recv, há espaço.
    assert_eq!(kata_rt_channel_recv(handle), 1);
    assert_eq!(
        kata_rt_channel_send(handle, 2),
        OK,
        "send após recv deve OK"
    );
    kata_rt_arena_destroy(arena);
}

// ── Broadcast (pub-sub) ──────────────────────────────────────────

#[test]
fn broadcast_create_returns_valid_handle() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_broadcast_create(arena);
    assert_ne!(handle, 0, "handle não deve ser 0");
    assert_eq!(handle & 0b11, 0b10, "tag deve ser 0b10 (broadcast)");
    kata_rt_arena_destroy(arena);
}

#[test]
fn broadcast_send_returns_ok() {
    let arena = kata_rt_arena_create();
    let handle = kata_rt_broadcast_create(arena);
    assert_eq!(
        kata_rt_channel_send(handle, 99),
        OK,
        "broadcast send sempre retorna OK"
    );
    kata_rt_arena_destroy(arena);
}

#[test]
fn broadcast_receiver_sees_future_messages_only() {
    let arena = kata_rt_arena_create();
    let bc = kata_rt_broadcast_create(arena);
    // Envia antes de criar receiver — receiver NÃO deve ver.
    kata_rt_channel_send(bc, 100);
    // Cria receiver DEPOIS do send.
    let rx = kata_rt_broadcast_receiver_create(arena, bc);
    assert_ne!(rx, 0, "receiver handle não deve ser 0");
    assert_eq!(rx & 0b11, 0b11, "tag deve ser 0b11 (broadcast receiver)");
    // Receiver não vê mensagem anterior à sua criação (Decisão F).
    assert_eq!(
        kata_rt_channel_recv(rx),
        WOULD_BLOCK,
        "receiver não deve ver mensagens anteriores à sua criação"
    );
    // Envia nova mensagem — receiver DEVE ver.
    kata_rt_channel_send(bc, 200);
    assert_eq!(
        kata_rt_channel_recv(rx),
        200,
        "receiver deve ver mensagens futuras"
    );
    // Segundo recv sem nova mensagem — WOULD_BLOCK.
    assert_eq!(
        kata_rt_channel_recv(rx),
        WOULD_BLOCK,
        "recv sem nova mensagem deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(arena);
}

#[test]
fn broadcast_multiple_receivers_independent() {
    let arena = kata_rt_arena_create();
    let bc = kata_rt_broadcast_create(arena);
    let rx1 = kata_rt_broadcast_receiver_create(arena, bc);
    let rx2 = kata_rt_broadcast_receiver_create(arena, bc);
    // Ambos recebem a mesma mensagem futura.
    kata_rt_channel_send(bc, 42);
    assert_eq!(kata_rt_channel_recv(rx1), 42, "rx1 recebe 42");
    assert_eq!(kata_rt_channel_recv(rx2), 42, "rx2 recebe 42");
    // rx1 já consumiu — segunda tentativa WOULD_BLOCK.
    assert_eq!(kata_rt_channel_recv(rx1), WOULD_BLOCK);
    // rx2 também.
    assert_eq!(kata_rt_channel_recv(rx2), WOULD_BLOCK);
    kata_rt_arena_destroy(arena);
}

#[test]
fn broadcast_latest_only() {
    let arena = kata_rt_arena_create();
    let bc = kata_rt_broadcast_create(arena);
    let rx = kata_rt_broadcast_receiver_create(arena, bc);
    // Envia múltiplas mensagens sem recv entre elas.
    kata_rt_channel_send(bc, 1);
    kata_rt_channel_send(bc, 2);
    kata_rt_channel_send(bc, 3);
    // Receiver só vê a última (latest only, Decisão F).
    assert_eq!(
        kata_rt_channel_recv(rx),
        3,
        "receiver deve ver apenas a última mensagem"
    );
    kata_rt_arena_destroy(arena);
}

// ── Select ───────────────────────────────────────────────────────

#[test]
fn select_returns_from_first_ready() {
    let arena = kata_rt_arena_create();
    let ch1 = kata_rt_channel_create(arena);
    let ch2 = kata_rt_channel_create(arena);
    // ch1 tem dado, ch2 não.
    kata_rt_channel_send(ch1, 42);
    let handles = [ch1, ch2];
    let result = kata_rt_select(handles.as_ptr(), 2);
    assert_ne!(result, WOULD_BLOCK, "select deve encontrar ch1 pronto");
    let idx = (result >> 32) as i64;
    let val = result & 0xFFFF_FFFF;
    assert_eq!(idx, 0, "índice deve ser 0 (ch1)");
    assert_eq!(val, 42, "valor deve ser 42");
    kata_rt_arena_destroy(arena);
}

#[test]
fn select_returns_would_block_when_all_empty() {
    let arena = kata_rt_arena_create();
    let ch1 = kata_rt_channel_create(arena);
    let ch2 = kata_rt_channel_create(arena);
    let handles = [ch1, ch2];
    let result = kata_rt_select(handles.as_ptr(), 2);
    assert_eq!(
        result, WOULD_BLOCK,
        "select com todos vazios deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(arena);
}

#[test]
fn select_skips_empty_finds_ready() {
    let arena = kata_rt_arena_create();
    let ch1 = kata_rt_channel_create(arena);
    let ch2 = kata_rt_channel_create(arena);
    // ch2 tem dado, ch1 não.
    kata_rt_channel_send(ch2, 99);
    let handles = [ch1, ch2];
    let result = kata_rt_select(handles.as_ptr(), 2);
    assert_ne!(result, WOULD_BLOCK);
    let idx = (result >> 32) as i64;
    let val = result & 0xFFFF_FFFF;
    assert_eq!(idx, 1, "índice deve ser 1 (ch2)");
    assert_eq!(val, 99, "valor deve ser 99");
    kata_rt_arena_destroy(arena);
}

// ── Tag dispatch ─────────────────────────────────────────────────

#[test]
fn send_on_broadcast_receiver_returns_would_block() {
    let arena = kata_rt_arena_create();
    let bc = kata_rt_broadcast_create(arena);
    let rx = kata_rt_broadcast_receiver_create(arena, bc);
    // Receiver não pode enviar.
    assert_eq!(
        kata_rt_channel_send(rx, 42),
        WOULD_BLOCK,
        "send em receiver deve WOULD_BLOCK (tag inválida para send)"
    );
    kata_rt_arena_destroy(arena);
}

#[test]
fn recv_on_broadcast_sender_returns_would_block() {
    let arena = kata_rt_arena_create();
    let bc = kata_rt_broadcast_create(arena);
    // Sender não pode receber.
    assert_eq!(
        kata_rt_channel_recv(bc),
        WOULD_BLOCK,
        "recv em sender deve WOULD_BLOCK (tag inválida para recv)"
    );
    kata_rt_arena_destroy(arena);
}

#[test]
fn broadcast_receiver_create_with_wrong_tag_returns_zero() {
    let arena = kata_rt_arena_create();
    let ch = kata_rt_channel_create(arena); // tag 0b00, não broadcast
    let result = kata_rt_broadcast_receiver_create(arena, ch);
    assert_eq!(
        result, 0,
        "receiver_create com handle não-broadcast deve retornar 0"
    );
    kata_rt_arena_destroy(arena);
}
