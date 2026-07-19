//! Telemetria via CSP — FFIs de log (Fio 14 `@log`).
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
    kata_rt_broadcast_create, kata_rt_channel_recv, kata_rt_channel_send, kata_rt_queue_create,
};

// ── LogConfig TLS ───────────────────────────────────────────
//
// Defaults de logging para o fiber atual. Herdado via snapshot no
// `kata_rt_spawn` (copia do pai para o filho). `topic` e `policy` são String
// owned (clone no spawn — aceitável pois não é hot path). `level` é i64 (tag
// do enum LogLevel: 0=Debug, 1=Info, 2=Warn, 3=Error).

/// Config de logging de um fiber.
#[derive(Clone, Default)]
pub struct LogConfig {
    pub topic: Option<String>,
    pub policy: Option<String>,
    pub level: Option<i64>,
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

/// Reseta o estado de log entre execuções. Chamado por `reset_scheduler`.
pub fn reset_log() {
    LOG_CONFIG.with(|c| {
        c.borrow_mut().take();
    });
    TOPIC_REGISTRY.with(|r| {
        r.borrow_mut().clear();
    });
}

/// Copia `LOG_CONFIG` do fiber pai para o filho (snapshot β).
/// Chamada por `kata_rt_spawn`.
pub fn snapshot_log_config() -> Option<LogConfig> {
    LOG_CONFIG.with(|c| c.borrow().clone())
}

/// Restaura `LOG_CONFIG` a partir do snapshot do fiber.
/// Chamada pelo scheduler antes de resumir um fiber.
pub fn restore_log_config(cfg: Option<LogConfig>) {
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
fn get_or_create_topic(topic: &str, policy: &str) -> i64 {
    TOPIC_REGISTRY.with(|r| {
        let registry = r.borrow();
        if let Some(&handle) = registry.get(topic) {
            return handle;
        }
        drop(registry);
        // Cria o canal na arena raiz.
        let arena = kata_rt_arena_create();
        let handle = if policy == "block" {
            kata_rt_queue_create(arena, 1)
        } else {
            // "drop" ou default → Broadcast
            kata_rt_broadcast_create(arena)
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
        read_text(policy_ptr)
    } else {
        LOG_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.policy.clone())
                .unwrap_or_else(|| "drop".to_string())
        })
    };

    let handle = get_or_create_topic(&topic, &policy);
    // Envia a mensagem no canal. Para "block", channel_send suspende se cheio.
    // Para "drop" (Broadcast), channel_send é fire-and-forget.
    let _ = kata_rt_channel_send(handle, msg);
    0
}

/// `kata_rt_log_recv(topic_ptr) -> i64`
///
/// Recebe a próxima mensagem de telemetria do tópico. Bloqueia (yield point)
/// se vazio. Retorna o valor (handle Text) ou 0 se canal fechou.
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
        Some(h) => kata_rt_channel_recv(h),
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
            cfg.policy = Some(read_text(policy_ptr));
        }
        cfg.level = Some(level);
        *c.borrow_mut() = Some(cfg);
    });
}
