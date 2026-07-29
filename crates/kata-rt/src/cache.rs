//! Cache de memoização para funções `@cache{strategy: "LRU"}`.
//!
//! O codegen emite cache lookup no prólogo da função e insert no epílogo.
//! O cache é armazenado em TLS, indexado por `fn_id`. Os valores cacheados
//! são ponteiros (i64) para dados na caller_arena — quando a arena morre
//! (fiber termina ou root no fim do run), os valores morrem junto.
//!
//! `kata_rt_cache_get_or_create(arena, fn_id, capacity)` → handle (fn_id)
//! `kata_rt_cache_lookup(handle, key_bytes, key_len)` → 0=miss, ptr=hit
//! `kata_rt_cache_insert(handle, key_bytes, key_len, value)`

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CACHES: RefCell<HashMap<i64, CacheTable>> = RefCell::new(HashMap::new());
}

struct CacheTable {
    /// Mapa de key_bytes → value (ponteiro para arena).
    entries: HashMap<Vec<u8>, i64>,
    /// Capacidade máxima (LRU eviction quando excedida).
    capacity: usize,
    /// Contador de acesso global para LRU.
    access_counter: i64,
    /// Último acesso de cada key (para LRU eviction).
    last_access: HashMap<Vec<u8>, i64>,
}

/// Cria ou retorna o handle do cache para `fn_id`.
///
/// O `arena_handle` é aceito para futura implementação arena-allocated,
/// mas atualmente o cache vive em TLS (Rust heap). O `capacity` define
/// o número máximo de entradas antes de LRU eviction.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_cache_get_or_create(
    _arena_handle: i64,
    fn_id: i64,
    capacity: i64,
) -> i64 {
    CACHES.with(|caches| {
        let mut caches = caches.borrow_mut();
        if !caches.contains_key(&fn_id) {
            let cap = if capacity > 0 { capacity as usize } else { 256 };
            caches.insert(
                fn_id,
                CacheTable {
                    entries: HashMap::new(),
                    capacity: cap,
                    access_counter: 0,
                    last_access: HashMap::new(),
                },
            );
        }
    });
    fn_id
}

/// Procura uma key no cache. Retorna 0 se miss, ou o valor (ponteiro) se hit.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_cache_lookup(handle: i64, key_ptr: i64, key_len: i64) -> i64 {
    if key_len <= 0 || key_ptr == 0 {
        return 0;
    }
    let key =
        unsafe { std::slice::from_raw_parts(key_ptr as *const u8, key_len as usize) }.to_vec();
    CACHES.with(|caches| {
        let mut caches = caches.borrow_mut();
        if let Some(table) = caches.get_mut(&handle) {
            if let Some(&val) = table.entries.get(&key) {
                table.access_counter += 1;
                table.last_access.insert(key, table.access_counter);
                return val;
            }
        }
        0
    })
}

/// Insere um valor no cache. Se o cache está cheio, evicta a entrada LRU.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_cache_insert(handle: i64, key_ptr: i64, key_len: i64, value: i64) {
    if key_len <= 0 || key_ptr == 0 {
        return;
    }
    let key =
        unsafe { std::slice::from_raw_parts(key_ptr as *const u8, key_len as usize) }.to_vec();
    CACHES.with(|caches| {
        let mut caches = caches.borrow_mut();
        if let Some(table) = caches.get_mut(&handle) {
            // LRU eviction se cache cheio e key é nova.
            if !table.entries.contains_key(&key) && table.entries.len() >= table.capacity {
                if let Some(lru_key) = table
                    .last_access
                    .iter()
                    .min_by_key(|(_, ts)| *ts)
                    .map(|(k, _)| k.clone())
                {
                    table.entries.remove(&lru_key);
                    table.last_access.remove(&lru_key);
                }
            }
            table.access_counter += 1;
            table.entries.insert(key.clone(), value);
            table.last_access.insert(key, table.access_counter);
        }
    });
}

/// Reseta todos os caches — chamado entre execuções de teste.
#[allow(dead_code)]
pub(crate) fn reset_caches() {
    CACHES.with(|caches| {
        caches.borrow_mut().clear();
    });
}
