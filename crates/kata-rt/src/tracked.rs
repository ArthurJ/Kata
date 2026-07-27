//! Tracked ARC — header {refcount, size, destructor} prefixando dados na root_arena.
//!
//! Para valores com `EscapeTarget::Heap` (enviados via canal), o codegen aloca
//! via `kata_rt_alloc_tracked` em vez de `kata_rt_arena_alloc`. O header permite
//! `incref`/`decref` com deallocation individual quando refcount → 0.
//!
//! ```text
//! Layout do bloco na root_arena:
//! offset 0:  refcount (i64) — contagem de referências (sempre ≥1)
//! offset 8:  size (i64) — tamanho total do bloco (header + dados)
//! offset 16: destructor_fn_ptr (i64) — 0 = leaf (sem children), non-null = tem children
//! offset 24: data[0..size-24] — os dados do valor (tupla, struct, array, etc.)
//! ```
//!
//! O codegen recebe `ptr + 24` (data_ptr). Loads/stores acessam dados normalmente.
//! `incref(data_ptr)` e `decref(data_ptr)` leem/escrevem o header atrás do ptr.
//!
//! `decref` quando refcount → 0:
//! 1. Se `destructor != 0`: chama `destructor(data_ptr)` — percorre children
//! 2. `kata_rt_arena_dealloc(root_arena, data_ptr - 24, size)` — libera o bloco inteiro

/// Tamanho do header (refcount + size + destructor).
const TRACKED_HEADER_SIZE: i64 = 24;

/// Aloca um bloco tracked na root_arena com header ARC.
///
/// `data_size` é o tamanho dos dados (sem o header). O bloco total é
/// `data_size + 24`. Retorna `data_ptr` (ponteiro para os dados, pulando
/// o header). O caller usa `data_ptr` normalmente — loads/stores não
/// enxergam o header.
///
/// `destructor_fn_ptr` é 0 para leaf (sem children ARC) ou um ponteiro
/// de função C-ABI `fn(data_ptr: i64)` que percorre children e faz
/// `decref_tracked` em cada um.
///
/// # Safety
/// `root_arena_handle` deve ser um handle válido de arena Tracked.
/// `data_size` deve ser > 0.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_alloc_tracked(
    root_arena_handle: i64,
    data_size: i64,
    destructor_fn_ptr: i64,
) -> i64 {
    if data_size <= 0 || root_arena_handle < 0 {
        return 0;
    }

    let total_size = data_size + TRACKED_HEADER_SIZE;
    let block_ptr = crate::arena::kata_rt_arena_alloc(root_arena_handle, total_size);
    if block_ptr == 0 {
        return 0;
    }

    unsafe {
        let ptr = block_ptr as *mut u8;
        // refcount = 1 no offset 0
        std::ptr::write_unaligned(ptr as *mut i64, 1);
        // size = total_size no offset 8
        std::ptr::write_unaligned(ptr.add(8) as *mut i64, total_size);
        // destructor_fn_ptr no offset 16
        std::ptr::write_unaligned(ptr.add(16) as *mut i64, destructor_fn_ptr);
    }

    // Retorna data_ptr = block_ptr + 24 (pula o header)
    block_ptr + TRACKED_HEADER_SIZE
}

/// Incrementa o refcount de um valor tracked.
///
/// `data_ptr` é o ponteiro retornado por `kata_rt_alloc_tracked` (aponta
/// para os dados, não para o header). A função lê o refcount 24 bytes atrás.
///
/// # Safety
/// `data_ptr` deve ser um ponteiro válido retornado por `kata_rt_alloc_tracked`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_incref_tracked(data_ptr: i64) -> i64 {
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        let header_ptr = (data_ptr - TRACKED_HEADER_SIZE) as *mut u8;
        let refcount_ptr = header_ptr as *mut i64;
        let count = std::ptr::read_unaligned(refcount_ptr);
        std::ptr::write_unaligned(refcount_ptr, count + 1);
    }
    0
}

/// Decrementa o refcount de um valor tracked. Quando refcount → 0:
/// 1. Se destructor != 0: chama `destructor(data_ptr)` para decref de children
/// 2. Desaloca o bloco inteiro (header + dados) da root_arena
///
/// # Safety
/// `data_ptr` deve ser um ponteiro válido retornado por `kata_rt_alloc_tracked`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_decref_tracked(data_ptr: i64) -> i64 {
    if data_ptr == 0 {
        return 0;
    }
    unsafe {
        let header_ptr = (data_ptr - TRACKED_HEADER_SIZE) as *mut u8;
        let refcount_ptr = header_ptr as *mut i64;
        let count = std::ptr::read_unaligned(refcount_ptr);
        if count > 0 {
            let new_count = count - 1;
            std::ptr::write_unaligned(refcount_ptr, new_count);
            if new_count == 0 {
                // Lê size e destructor do header
                let size = std::ptr::read_unaligned(header_ptr.add(8) as *mut i64);
                let destructor = std::ptr::read_unaligned(header_ptr.add(16) as *mut i64);

                // 1. Se tem destructor, chama para decref de children
                if destructor != 0 {
                    let dtor: extern "C" fn(i64) = std::mem::transmute(destructor as *const u8);
                    dtor(data_ptr);
                }

                // 2. Desaloca o bloco inteiro (header + dados)
                let root_arena = crate::arena::kata_rt_get_root_arena_handle();
                if root_arena != 0 {
                    let block_ptr = data_ptr - TRACKED_HEADER_SIZE;
                    crate::arena::kata_rt_arena_dealloc(root_arena, block_ptr, size);
                }
            }
        }
    }
    0
}
