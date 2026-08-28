//! Cache de memoização para funções `@cache{strategy: "LRU", capacity: 256}`.
//!
//! O codegen emite cache lookup no prólogo da função e insert no epílogo.
//! O cache é armazenado em TLS, indexado por `fn_id`. Os valores cacheados
//! são ponteiros (i64) para dados na caller_arena — quando a arena morre
//! (fiber termina ou root no fim do run), os valores morrem junto.
//!
//! Estratégias de eviction: LRU, FIFO, MRU, LFU.
//!
//! `kata_rt_cache_get_or_create(arena, fn_id, capacity, strategy_tag)` → handle
//! `kata_rt_cache_lookup(handle, key_bytes, key_len)` → 0=miss, ptr=hit
//! `kata_rt_cache_insert(handle, key_bytes, key_len, value)`

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CACHES: RefCell<HashMap<i64, CacheTable>> = RefCell::new(HashMap::new());
}

/// Estratégia de eviction do cache.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CacheStrategy {
    LRU = 0,
    FIFO = 1,
    MRU = 2,
    LFU = 3,
}

struct CacheTable {
    /// Mapa de key_bytes → value (ponteiro para arena).
    entries: HashMap<Vec<u8>, i64>,
    /// Capacidade máxima (eviction quando excedida).
    capacity: usize,
    /// Estratégia de eviction.
    strategy: CacheStrategy,
    /// Contador global de acessos (lookup + insert).
    access_counter: i64,
    /// Último acesso de cada key (para LRU/MRU).
    last_access: HashMap<Vec<u8>, i64>,
    /// Ordem de inserção de cada key (para FIFO).
    insert_order: HashMap<Vec<u8>, i64>,
    /// Contador de inserção (para FIFO).
    insert_counter: i64,
    /// Contagem de acessos por key (para LFU).
    access_count: HashMap<Vec<u8>, i64>,
}

/// Cria ou retorna o handle do cache para `fn_id`.
///
/// O `arena_handle` é aceito para futura implementação arena-allocated,
/// mas atualmente o cache vive em TLS (Rust heap). O `capacity` define
/// o número máximo de entradas antes de eviction. O `strategy_tag` define
/// a política de eviction: 0=LRU, 1=FIFO, 2=MRU, 3=LFU.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_cache_get_or_create(
    _arena_handle: i64,
    fn_id: i64,
    capacity: i64,
    strategy_tag: i64,
) -> i64 {
    CACHES.with(|caches| {
        let mut caches = caches.borrow_mut();
        caches.entry(fn_id).or_insert_with(|| {
            let cap = if capacity > 0 { capacity as usize } else { 256 };
            let strategy = match strategy_tag {
                1 => CacheStrategy::FIFO,
                2 => CacheStrategy::MRU,
                3 => CacheStrategy::LFU,
                _ => CacheStrategy::LRU,
            };
            CacheTable {
                entries: HashMap::new(),
                capacity: cap,
                strategy,
                access_counter: 0,
                last_access: HashMap::new(),
                insert_order: HashMap::new(),
                insert_counter: 0,
                access_count: HashMap::new(),
            }
        });
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
        if let Some(table) = caches.get_mut(&handle)
            && let Some(&val) = table.entries.get(&key)
        {
            // Atualiza metadata conforme estratégia.
            match table.strategy {
                CacheStrategy::LRU | CacheStrategy::MRU => {
                    table.access_counter += 1;
                    table.last_access.insert(key, table.access_counter);
                }
                CacheStrategy::LFU => {
                    *table.access_count.get_mut(&key).unwrap() += 1;
                }
                CacheStrategy::FIFO => {}
            }
            return val;
        }
        0
    })
}

/// Insere um valor no cache. Se o cache está cheio, evicta conforme estratégia.
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
            // Eviction se cache cheio e key é nova.
            if !table.entries.contains_key(&key) && table.entries.len() >= table.capacity {
                if let Some(victim) = evict(table) {
                    table.entries.remove(&victim);
                    table.last_access.remove(&victim);
                    table.insert_order.remove(&victim);
                    table.access_count.remove(&victim);
                }
            }
            table.entries.insert(key.clone(), value);
            // Atualiza metadata conforme estratégia.
            match table.strategy {
                CacheStrategy::LRU | CacheStrategy::MRU => {
                    table.access_counter += 1;
                    table.last_access.insert(key, table.access_counter);
                }
                CacheStrategy::FIFO => {
                    table.insert_counter += 1;
                    table.insert_order.insert(key, table.insert_counter);
                }
                CacheStrategy::LFU => {
                    table.access_count.insert(key, 1);
                }
            }
        }
    });
}

/// Encontra a key vítima para eviction conforme a estratégia.
fn evict(table: &CacheTable) -> Option<Vec<u8>> {
    match table.strategy {
        CacheStrategy::LRU => {
            // Evicta menor last_access (menos recentemente acessada).
            table
                .last_access
                .iter()
                .min_by_key(|(_, ts)| *ts)
                .map(|(k, _)| k.clone())
        }
        CacheStrategy::FIFO => {
            // Evicta menor insert_order (primeira inserida).
            table
                .insert_order
                .iter()
                .min_by_key(|(_, order)| *order)
                .map(|(k, _)| k.clone())
        }
        CacheStrategy::MRU => {
            // Evicta maior last_access (mais recentemente acessada).
            table
                .last_access
                .iter()
                .max_by_key(|(_, ts)| *ts)
                .map(|(k, _)| k.clone())
        }
        CacheStrategy::LFU => {
            // Evicta menor access_count (menos frequentemente acessada).
            table
                .access_count
                .iter()
                .min_by_key(|(_, count)| *count)
                .map(|(k, _)| k.clone())
        }
    }
}

// ── Serialização de cache key por conteúdo ─────────────────────────
//
// Type descriptor tags (C-ABI estável, bytes):
const TD_INT: u8 = 0x01;
const TD_FLOAT: u8 = 0x02;
const TD_TEXT: u8 = 0x03;
const TD_LIST: u8 = 0x04;
const TD_STRUCT: u8 = 0x05;
const TD_TUPLE: u8 = 0x06;
const TD_SUM: u8 = 0x07;
const TD_UNIT: u8 = 0x00;

/// Serializa um valor i64 segundo um type descriptor, escrevendo bytes
/// de conteúdo (não ponteiros) num buffer de saída.
///
/// `value` é o valor i64 (imediato para escalares, ponteiro para heap types).
/// `desc` é o type descriptor (byte array construído pelo codegen).
/// `out` é o buffer de saída. `out_cap` é a capacidade.
///
/// Retorna o número de bytes escritos, ou -1 se o buffer estourou.
///
/// O type descriptor é caminhado recursivamente. Para tipos complexos,
/// segue ponteiros na arena e copia conteúdo. Para Text, copia os bytes
/// da C string. Para List, percorre cons cells. Para Struct/Tuple, lê
/// campos de 8 bytes cada. Para Sum, lê tag + payload.
///
/// Dois valores estruturalmente iguais produzem os mesmos bytes.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_serialize_key(
    value: i64,
    desc_ptr: i64,
    desc_len: i64,
    out_ptr: i64,
    out_cap: i64,
) -> i64 {
    if desc_ptr == 0 || desc_len <= 0 || out_ptr == 0 || out_cap <= 0 {
        return -1;
    }
    let desc = unsafe { std::slice::from_raw_parts(desc_ptr as *const u8, desc_len as usize) };
    let out = unsafe { std::slice::from_raw_parts_mut(out_ptr as *mut u8, out_cap as usize) };
    let mut pos = 0usize;
    match serialize_value(value, desc, &mut 0, out, &mut pos) {
        Ok(()) => pos as i64,
        Err(()) => -1,
    }
}

/// Serializa um valor recursivamente segundo o descriptor.
/// `desc_pos` é a posição atual no descriptor.
/// `out` é o buffer de saída, `out_pos` é a posição atual.
fn serialize_value(
    value: i64,
    desc: &[u8],
    desc_pos: &mut usize,
    out: &mut [u8],
    out_pos: &mut usize,
) -> Result<(), ()> {
    if *desc_pos >= desc.len() {
        return Err(());
    }
    let tag = desc[*desc_pos];
    *desc_pos += 1;
    match tag {
        TD_INT | TD_FLOAT => {
            // Escalares: 8 bytes do valor imediato.
            if *out_pos + 8 > out.len() {
                return Err(());
            }
            out[*out_pos..*out_pos + 8].copy_from_slice(&value.to_le_bytes());
            *out_pos += 8;
        }
        TD_TEXT => {
            // Text: C string nulo-terminada. Copiar bytes + nul.
            if value == 0 {
                // Null string = empty.
                if *out_pos + 1 > out.len() {
                    return Err(());
                }
                out[*out_pos] = 0;
                *out_pos += 1;
            } else {
                let s = unsafe {
                    std::ffi::CStr::from_ptr(value as *const std::os::raw::c_char).to_bytes()
                };
                let len = s.len();
                // Escrever len (4 bytes LE) + bytes da string.
                if *out_pos + 4 + len > out.len() {
                    return Err(());
                }
                out[*out_pos..*out_pos + 4].copy_from_slice(&(len as u32).to_le_bytes());
                *out_pos += 4;
                out[*out_pos..*out_pos + len].copy_from_slice(s);
                *out_pos += len;
            }
        }
        TD_LIST => {
            // List: percorre cons cells (head: i64, tail: i64).
            // Descriptor do elemento segue o TD_LIST tag.
            let elem_desc_start = *desc_pos;
            let mut current = value;
            while current != 0 {
                let ptr = current as *const u8;
                let head = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
                let tail = unsafe { std::ptr::read_unaligned(ptr.add(8) as *const i64) };
                // Serializar head com o descriptor do elemento.
                *desc_pos = elem_desc_start;
                serialize_value(head, desc, desc_pos, out, out_pos)?;
                current = tail;
            }
            // Marcador de fim de lista: 0 (distingue [1] de [1 1] etc.)
            if *out_pos + 1 > out.len() {
                return Err(());
            }
            out[*out_pos] = 0;
            *out_pos += 1;
        }
        TD_STRUCT => {
            // Struct: n_fields (u8) + field descriptors.
            if *desc_pos >= desc.len() {
                return Err(());
            }
            let n_fields = desc[*desc_pos] as usize;
            *desc_pos += 1;
            let ptr = value as *const u8;
            for i in 0..n_fields {
                let field_val = unsafe { std::ptr::read_unaligned(ptr.add(i * 8) as *const i64) };
                serialize_value(field_val, desc, desc_pos, out, out_pos)?;
            }
        }
        TD_TUPLE => {
            // Tuple: n_elems (u8) + elem descriptors.
            if *desc_pos >= desc.len() {
                return Err(());
            }
            let n_elems = desc[*desc_pos] as usize;
            *desc_pos += 1;
            let ptr = value as *const u8;
            for i in 0..n_elems {
                let elem_val = unsafe { std::ptr::read_unaligned(ptr.add(i * 8) as *const i64) };
                serialize_value(elem_val, desc, desc_pos, out, out_pos)?;
            }
        }
        TD_SUM => {
            // Sum: n_variants (u8) + payload descriptors (um por variant).
            // Layout: tag (i64 offset 0) + payload (i64 offset 8).
            // Serializamos tag (8 bytes) + payload segundo o descriptor da variant.
            if *desc_pos >= desc.len() {
                return Err(());
            }
            let n_variants = desc[*desc_pos] as usize;
            *desc_pos += 1;
            let ptr = value as *const u8;
            let tag = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
            // Escrever tag (8 bytes).
            if *out_pos + 8 > out.len() {
                return Err(());
            }
            out[*out_pos..*out_pos + 8].copy_from_slice(&tag.to_le_bytes());
            *out_pos += 8;
            // Avançar o descriptor para o payload da variant correta.
            let payload_desc_start =
                advance_to_variant_payload(desc, desc_pos, tag as usize, n_variants);
            let payload = unsafe { std::ptr::read_unaligned(ptr.add(8) as *const i64) };
            if payload_desc_start < desc.len() {
                *desc_pos = payload_desc_start;
                serialize_value(payload, desc, desc_pos, out, out_pos)?;
            } else {
                // Variante sem payload — escrever 0.
                if *out_pos + 8 > out.len() {
                    return Err(());
                }
                out[*out_pos..*out_pos + 8].copy_from_slice(&0i64.to_le_bytes());
                *out_pos += 8;
            }
        }
        TD_UNIT => {
            // Unit: 0 bytes. Nada a escrever.
        }
        _ => return Err(()),
    }
    Ok(())
}

/// Avança o descriptor para o início do payload da variant `tag`.
/// O descriptor após TD_SUM tem: n_variants (u8) + [variant_desc...] onde
/// cada variant_desc é um sub-descriptor completo. Percorre `tag` variants.
fn advance_to_variant_payload(
    desc: &[u8],
    pos: &mut usize,
    tag: usize,
    n_variants: usize,
) -> usize {
    let start = *pos;
    for i in 0..n_variants {
        if i == tag {
            return *pos;
        }
        // Avançar um sub-descriptor completo.
        skip_descriptor(desc, pos);
    }
    start // fallback
}

/// Avança `pos` sobre um sub-descriptor completo (recursivo).
fn skip_descriptor(desc: &[u8], pos: &mut usize) {
    if *pos >= desc.len() {
        return;
    }
    let tag = desc[*pos];
    *pos += 1;
    match tag {
        TD_INT | TD_FLOAT | TD_TEXT | TD_UNIT => {}
        TD_LIST => {
            skip_descriptor(desc, pos);
        }
        TD_STRUCT => {
            if *pos >= desc.len() {
                return;
            }
            let n = desc[*pos] as usize;
            *pos += 1;
            for _ in 0..n {
                skip_descriptor(desc, pos);
            }
        }
        TD_TUPLE => {
            if *pos >= desc.len() {
                return;
            }
            let n = desc[*pos] as usize;
            *pos += 1;
            for _ in 0..n {
                skip_descriptor(desc, pos);
            }
        }
        TD_SUM => {
            if *pos >= desc.len() {
                return;
            }
            let n = desc[*pos] as usize;
            *pos += 1;
            for _ in 0..n {
                skip_descriptor(desc, pos);
            }
        }
        _ => {}
    }
}
