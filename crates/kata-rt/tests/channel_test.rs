//! Testes unitários do runtime de canais CSP.
//!
//! Sem JIT — testa as FFI functions diretamente. Cada teste cria uma
//! arena via `kata_rt_arena_create`, passa para as funções de criação
//! de canal, e verifica comportamento.
//!
//! Sem blocking real. `WOULD_BLOCK` (-1) é retornado quando
//! a operação não pode completar. A versão com yield substitui por blocking cooperativo.

use kata_rt::{Runtime, kata_rt_arena_create, kata_rt_arena_destroy, kata_rt_broadcast_create,
    kata_rt_broadcast_receiver_create, kata_rt_channel_create, kata_rt_channel_recv,
    kata_rt_channel_send, kata_rt_queue_create, kata_rt_select};

const WOULD_BLOCK: i64 = -1;
const OK: i64 = 0;

/// Cria um Runtime para o teste e retorna o ponteiro `i64` a ser passado
/// como primeiro argumento (`rt`) às FFIs migradas para a A2.
fn make_rt() -> i64 {
    let rt = Box::new(Runtime::new());
    let ptr = Box::into_raw(rt) as i64;
    // FFIs periféricas (channel/queue/broadcast/select) usam o cache TLS RT_PTR,
    // que é populado por scheduler_init. Como esses testes não chamam
    // scheduler_init, setamos o cache explicitamente.
    kata_rt::set_rt_ptr(ptr);
    ptr
}

/// Descarta o Runtime criado por `make_rt`.
fn drop_rt(rt_ptr: i64) {
    unsafe { drop(Box::from_raw(rt_ptr as *mut Runtime)) };
}

// ── Channel (rendezvous) ──────────────────────────────────────────

#[test]
fn channel_create_returns_valid_handle() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_channel_create(arena);
    assert_ne!(handle, 0, "handle não deve ser 0 (null)");
    // Tag 0b00 (channel) nos 2 bits baixos.
    assert_eq!(handle & 0b11, 0b00, "tag deve ser 0b00 (channel)");
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn channel_send_then_recv_succeeds() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_channel_create(arena);
    let status = kata_rt_channel_send(handle, 42);
    assert_eq!(status, OK, "send em canal vazio deve retornar OK");
    let val = kata_rt_channel_recv(handle);
    assert_eq!(val, 42, "recv deve retornar o valor enviado");
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn channel_recv_empty_returns_would_block() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_channel_create(arena);
    let val = kata_rt_channel_recv(handle);
    assert_eq!(
        val, WOULD_BLOCK,
        "recv em canal vazio deve retornar WOULD_BLOCK"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn channel_send_full_returns_would_block() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_channel_create(arena);
    let status1 = kata_rt_channel_send(handle, 10);
    assert_eq!(status1, OK, "primeiro send deve OK");
    let status2 = kata_rt_channel_send(handle, 20);
    assert_eq!(
        status2, WOULD_BLOCK,
        "segundo send (slot ocupado) deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

// ── Queue (buffered) ─────────────────────────────────────────────

#[test]
fn queue_create_returns_valid_handle() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_queue_create(arena, 3, 0);
    assert_ne!(handle, 0, "handle não deve ser 0");
    assert_eq!(handle & 0b11, 0b01, "tag deve ser 0b01 (queue)");
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn queue_send_recv_buffered() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_queue_create(arena, 3, 0);
    // Múltiplos sends cabem no buffer sem bloquear.
    assert_eq!(kata_rt_channel_send(handle, 1), OK);
    assert_eq!(kata_rt_channel_send(handle, 2), OK);
    assert_eq!(kata_rt_channel_send(handle, 3), OK);
    // FIFO.
    assert_eq!(kata_rt_channel_recv(handle), 1);
    assert_eq!(kata_rt_channel_recv(handle), 2);
    assert_eq!(kata_rt_channel_recv(handle), 3);
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn queue_full_returns_would_block() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_queue_create(arena, 2, 0);
    assert_eq!(kata_rt_channel_send(handle, 1), OK);
    assert_eq!(kata_rt_channel_send(handle, 2), OK);
    assert_eq!(
        kata_rt_channel_send(handle, 3),
        WOULD_BLOCK,
        "send com buffer cheio deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn queue_empty_returns_would_block() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_queue_create(arena, 2, 0);
    assert_eq!(
        kata_rt_channel_recv(handle),
        WOULD_BLOCK,
        "recv em queue vazia deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn queue_recv_makes_space_for_send() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_queue_create(arena, 1, 0);
    assert_eq!(kata_rt_channel_send(handle, 1), OK);
    assert_eq!(kata_rt_channel_send(handle, 2), WOULD_BLOCK);
    // Após recv, há espaço.
    assert_eq!(kata_rt_channel_recv(handle), 1);
    assert_eq!(
        kata_rt_channel_send(handle, 2),
        OK,
        "send após recv deve OK"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

// ── Broadcast (pub-sub) ──────────────────────────────────────────

#[test]
fn broadcast_create_returns_valid_handle() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_broadcast_create(arena);
    assert_ne!(handle, 0, "handle não deve ser 0");
    assert_eq!(handle & 0b11, 0b10, "tag deve ser 0b10 (broadcast)");
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn broadcast_send_returns_ok() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let handle = kata_rt_broadcast_create(arena);
    assert_eq!(
        kata_rt_channel_send(handle, 99),
        OK,
        "broadcast send sempre retorna OK"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn broadcast_receiver_sees_future_messages_only() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
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
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn broadcast_multiple_receivers_independent() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
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
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn broadcast_latest_only() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
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
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

// ── Select ───────────────────────────────────────────────────────

#[test]
fn select_returns_from_first_ready() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let ch1 = kata_rt_channel_create(arena);
    let ch2 = kata_rt_channel_create(arena);
    // ch1 tem dado, ch2 não.
    kata_rt_channel_send(ch1, 42);
    let handles = [ch1, ch2];
    let result = kata_rt_select(handles.as_ptr(), 2, -1);
    assert_ne!(result, WOULD_BLOCK, "select deve encontrar ch1 pronto");
    assert_eq!(result, 0, "índice deve ser 0 (ch1)");
    // Na nova semântica, select retorna só o índice. O valor é obtido
    // via channel_recv no canal selecionado.
    let val = kata_rt_channel_recv(ch1);
    assert_eq!(val, 42, "valor recebido deve ser 42");
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn select_returns_would_block_when_all_empty() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let ch1 = kata_rt_channel_create(arena);
    let ch2 = kata_rt_channel_create(arena);
    let handles = [ch1, ch2];
    let result = kata_rt_select(handles.as_ptr(), 2, -1);
    assert_eq!(
        result, WOULD_BLOCK,
        "select com todos vazios deve WOULD_BLOCK"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn select_skips_empty_finds_ready() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let ch1 = kata_rt_channel_create(arena);
    let ch2 = kata_rt_channel_create(arena);
    // ch2 tem dado, ch1 não.
    kata_rt_channel_send(ch2, 99);
    let handles = [ch1, ch2];
    let result = kata_rt_select(handles.as_ptr(), 2, -1);
    assert_ne!(result, WOULD_BLOCK);
    assert_eq!(result, 1, "índice deve ser 1 (ch2)");
    let val = kata_rt_channel_recv(ch2);
    assert_eq!(val, 99, "valor recebido deve ser 99");
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

// ── Tag dispatch ─────────────────────────────────────────────────

#[test]
fn send_on_broadcast_receiver_returns_would_block() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let bc = kata_rt_broadcast_create(arena);
    let rx = kata_rt_broadcast_receiver_create(arena, bc);
    // Receiver não pode enviar.
    assert_eq!(
        kata_rt_channel_send(rx, 42),
        WOULD_BLOCK,
        "send em receiver deve WOULD_BLOCK (tag inválida para send)"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn recv_on_broadcast_sender_returns_would_block() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let bc = kata_rt_broadcast_create(arena);
    // Sender não pode receber.
    assert_eq!(
        kata_rt_channel_recv(bc),
        WOULD_BLOCK,
        "recv em sender deve WOULD_BLOCK (tag inválida para recv)"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}

#[test]
fn broadcast_receiver_create_with_wrong_tag_returns_zero() {
    let rt = make_rt();
    let arena = kata_rt_arena_create(rt);
    let ch = kata_rt_channel_create(arena); // tag 0b00, não broadcast
    let result = kata_rt_broadcast_receiver_create(arena, ch);
    assert_eq!(
        result, 0,
        "receiver_create com handle não-broadcast deve retornar 0"
    );
    kata_rt_arena_destroy(rt, arena);
    drop_rt(rt);
}
