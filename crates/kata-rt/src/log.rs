//! Telemetria via CSP — FFIs de log (`@log`).
//!
//! `@log` e `log!()` publicam mensagens em tópicos (canais nomeados via
//! registry). Políticas: `"drop"` (Broadcast, fire-and-forget) ou `"block"`
//! (Queue bounded cap=1, bloqueia até consumo).
//!
//! `LOG_CONFIG` TLS carrega defaults herdados via snapshot no `kata_rt_spawn`.
//! O registry de tópicos é thread_local — resetado entre testes pelo
//! `reset_scheduler`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::c_char;

use crate::arena::kata_rt_arena_create;
use crate::channel::{
    Policy, kata_rt_broadcast_create, kata_rt_broadcast_receiver_create, kata_rt_channel_recv,
    kata_rt_channel_send, kata_rt_queue_create,
};

// ── LogConfig TLS ───────────────────────────────────────────
//
// Defaults de logging para o fiber atual. Herdado via snapshot no
// `kata_rt_spawn` (copia do pai para o filho). `topic` é String owned
// (clone no spawn — aceitável pois não é hot path). `policy` é enum
// `Policy` (Block ou Drop). `level` é i64 (tag do enum LogLevel:
// 0=Debug, 1=Info, 2=Warn, 3=Error).

/// Parse policy de string ("block" | "drop"). Default: Drop.
fn parse_policy(s: &str) -> Policy {
    match s {
        "block" => Policy::Block,
        _ => Policy::Drop,
    }
}

/// Config de logging de um fiber.
#[derive(Clone, Default)]
pub(crate) struct LogConfig {
    pub(crate) topic: Option<String>,
    pub(crate) policy: Option<Policy>,
    pub(crate) level: Option<i64>,
}

thread_local! {
    static LOG_CONFIG: RefCell<Option<LogConfig>> = const { RefCell::new(None) };
}

// ── Registry de tópicos ─────────────────────────────────────
//
// Mapeia nome do tópico → handle de canal. Criado sob demanda na primeira
// referência. Resetado entre testes pelo `reset_scheduler`.
//
// O handle do canal é alocado na arena raiz do scheduler. O registry só
// guarda o handle (i64) — não precisa de gestão de lifetime própria.

thread_local! {
    static TOPIC_REGISTRY: RefCell<HashMap<String, i64>> = RefCell::new(HashMap::new());
}

// Registry de receivers de broadcast por tópico.
//
// Para tópicos Broadcast (policy "drop"), `kata_rt_log_recv` precisa de um
// receiver (tag `TAG_BROADCAST_RX`) para consumir mensagens. O receiver é
// criado sob demanda na primeira `log_recv` para o tópico e cached aqui.
// Tópicos Queue (policy "block") não precisam — o handle do canal é usado
// diretamente.
thread_local! {
    static RECEIVER_REGISTRY: RefCell<HashMap<String, i64>> = RefCell::new(HashMap::new());
}

/// Reseta o estado de log entre execuções. Chamado por `reset_scheduler`.
pub(crate) fn reset_log() {
    LOG_CONFIG.with(|c| {
        c.borrow_mut().take();
    });
    TOPIC_REGISTRY.with(|r| {
        r.borrow_mut().clear();
    });
    RECEIVER_REGISTRY.with(|r| {
        r.borrow_mut().clear();
    });
}

/// Copia `LOG_CONFIG` do fiber pai para o filho (snapshot β).
/// Chamada por `kata_rt_spawn`.
pub(crate) fn snapshot_log_config() -> Option<LogConfig> {
    LOG_CONFIG.with(|c| c.borrow().clone())
}

/// Restaura `LOG_CONFIG` a partir do snapshot do fiber.
/// Chamada pelo scheduler antes de resumir um fiber.
pub(crate) fn restore_log_config(cfg: Option<LogConfig>) {
    LOG_CONFIG.with(|c| {
        *c.borrow_mut() = cfg;
    });
}

/// Lê uma string de um handle Text (ponteiro `*const c_char` castado para i64).
/// Retorna string vazia se `ptr == 0`.
fn read_text(ptr: i64) -> String {
    if ptr == 0 {
        return String::new();
    }
    unsafe {
        CStr::from_ptr(ptr as *const c_char)
            .to_string_lossy()
            .into_owned()
    }
}

/// Obtém ou cria o canal para um tópico. A política determina o tipo:
/// - `"drop"` → Broadcast (fire-and-forget)
/// - `"block"` → Queue bounded cap=1 (bloqueia até consumo)
///
/// O canal é alocado na arena raiz do scheduler (obtida via
/// `kata_rt_arena_create`). O registry guarda o handle.
fn get_or_create_topic(topic: &str, policy: Policy) -> i64 {
    TOPIC_REGISTRY.with(|r| {
        let registry = r.borrow();
        if let Some(&handle) = registry.get(topic) {
            return handle;
        }
        drop(registry);
        // Cria o canal na arena raiz.
        let arena = kata_rt_arena_create(crate::arena::rt_ptr());
        let handle = match policy {
            Policy::Block => kata_rt_queue_create(arena, 1, 0),
            Policy::Drop => {
                // Broadcast (fire-and-forget)
                let bh = kata_rt_broadcast_create(arena);
                // Eagerly cria um receiver para o tópico Broadcast, garantindo
                // que mensagens publicadas antes de qualquer log_recv! sejam
                // visíveis. O receiver começa com last_seen_version = 0 (version
                // atual = 0), então vê todas as mensagens futuras.
                if bh != 0 {
                    let rx = kata_rt_broadcast_receiver_create(arena, bh);
                    RECEIVER_REGISTRY.with(|rr| {
                        rr.borrow_mut().insert(topic.to_string(), rx);
                    });
                }
                bh
            }
        };
        r.borrow_mut().insert(topic.to_string(), handle);
        handle
    })
}

// ── FFIs C-ABI ──────────────────────────────────────────────

/// `kata_rt_log_publish(topic_ptr, level, msg, policy_ptr) -> i64`
///
/// Publica `msg` no tópico. `topic_ptr` e `policy_ptr` são handles para
/// strings Text na arena (ou 0 para usar config herdada). `level` é a tag
/// do enum LogLevel.
///
/// Retorna 0 (OK) ou -1 (erro).
///
/// NOTE: Esta FFI não bloqueia o fiber — se policy="block" e o canal está
/// cheio, `kata_rt_channel_send` suspende o fiber via `WaitingOnChannelSend`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_log_publish(
    topic_ptr: i64,
    _level: i64,
    msg: i64,
    policy_ptr: i64,
) -> i64 {
    // Resolve tópico: se topic_ptr != 0, usar a string apontada; senão, config.
    let topic = if topic_ptr != 0 {
        read_text(topic_ptr)
    } else {
        LOG_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.topic.clone())
                .unwrap_or_else(|| "default".to_string())
        })
    };

    let policy = if policy_ptr != 0 {
        parse_policy(&read_text(policy_ptr))
    } else {
        LOG_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.policy)
                .unwrap_or(Policy::Drop)
        })
    };

    // Tópicos especiais: stdout/stderr escrevem diretamente nas saídas padrão.
    // Não passam pelo canal CSP — útil para debug e telemetria de desenvolvimento.
    if topic == "stdout" || topic == "stderr" {
        let msg_text = read_text(msg);
        if topic == "stdout" {
            println!("{msg_text}");
        } else {
            eprintln!("{msg_text}");
        }
        return 0;
    }

    let handle = get_or_create_topic(&topic, policy);
    // Envia a mensagem no canal. Para "block", channel_send suspende se cheio.
    // Para "drop" (Broadcast), channel_send é fire-and-forget.
    let _ = kata_rt_channel_send(handle, msg);
    0
}

/// `kata_rt_log_recv(topic_ptr) -> i64`
///
/// Recebe a próxima mensagem de telemetria do tópico. Bloqueia (yield point)
/// se vazio. Retorna o valor (handle Text) ou 0 se canal fechou.
///
/// Para tópicos Broadcast (policy "drop"), obtém um receiver via
/// `kata_rt_broadcast_receiver_create` e o cacheia no `RECEIVER_REGISTRY`.
/// Para Queue (policy "block"), usa o handle diretamente.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_log_recv(topic_ptr: i64) -> i64 {
    let topic = if topic_ptr != 0 {
        read_text(topic_ptr)
    } else {
        LOG_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.topic.clone())
                .unwrap_or_else(|| "default".to_string())
        })
    };

    let handle = TOPIC_REGISTRY.with(|r| r.borrow().get(&topic).copied());

    match handle {
        Some(h) => {
            // Verifica se é Broadcast (tag 0b010). Se sim, precisa de receiver.
            let tag = h & 0b111;
            if tag == 0b010 {
                // Broadcast — obter ou criar receiver cached.
                let rx_handle = RECEIVER_REGISTRY.with(|r| {
                    if let Some(&rx) = r.borrow().get(&topic) {
                        return rx;
                    }
                    // Criar receiver na arena raiz.
                    let arena = kata_rt_arena_create(crate::arena::rt_ptr());
                    let rx = kata_rt_broadcast_receiver_create(arena, h);
                    r.borrow_mut().insert(topic.clone(), rx);
                    rx
                });
                if rx_handle == 0 {
                    return 0; // Falha ao criar receiver.
                }
                kata_rt_channel_recv(rx_handle)
            } else {
                // Queue ou Channel — recv direto.
                kata_rt_channel_recv(h)
            }
        }
        None => 0, // Tópico não existe → sem mensagem.
    }
}

/// `kata_rt_log_config(topic_ptr, policy_ptr, level) -> ()`
///
/// Setta defaults de logging para o fiber atual e descendentes (herdado
/// via snapshot no `kata_rt_spawn`).
///
/// `topic_ptr` e `policy_ptr` são handles para strings Text na arena.
/// Se 0, mantém o valor anterior (não sobrescreve).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_log_config(topic_ptr: i64, policy_ptr: i64, level: i64) {
    LOG_CONFIG.with(|c| {
        let mut cfg = c.borrow_mut().take().unwrap_or_default();
        if topic_ptr != 0 {
            cfg.topic = Some(read_text(topic_ptr));
        }
        if policy_ptr != 0 {
            cfg.policy = Some(parse_policy(&read_text(policy_ptr)));
        }
        cfg.level = Some(level);
        *c.borrow_mut() = Some(cfg);
    });
}
