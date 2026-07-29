//! Snapshots de valores comptime — carregados em load-time na root_arena.
//!
//! O comptime pass serializa valores complexos (List, Struct, Tuple, Text, Sum)
//! em bytes contíguos + rebase_offsets (offsets onde há ponteiros relativos).
//! O codegen embute esses bytes como data symbols no binário JIT.
//! O prólogo de `__kata_entry` chama `kata_rt_load_snapshot` para cada snapshot,
//! que copia os bytes para a root_arena e faz o rebasing (soma base_ptr a cada
//! offset em rebase_offsets). O resultado é armazenado numa tabela TLS.
//!
//! `kata_rt_get_snapshot(snapshot_id)` retorna o ponteiro da tabela TLS.
//! O codegen lowera `HeapSnapshot { snapshot_id, .. }` para esta chamada.

use std::cell::RefCell;

thread_local! {
    /// Tabela de ponteiros de snapshots carregados — indexada por snapshot_id.
    static SNAPSHOT_PTRS: RefCell<Vec<i64>> = RefCell::new(Vec::new());
}

/// Carrega um snapshot na root_arena e armazena o ponteiro na tabela TLS.
///
/// 1. Aloca `bytes_len` na root_arena → `base_ptr`
/// 2. `memcpy(base_ptr, bytes_ptr, bytes_len)`
/// 3. Para cada offset em `rebase_offsets`: `*(base_ptr + offset) += base_ptr`
/// 4. Armazena `base_ptr` em `SNAPSHOT_PTRS[snapshot_id]`
///
/// # Safety
/// `root_arena` deve ser um handle válido. `bytes_ptr` deve apontar para
/// `bytes_len` bytes válidos. `rebase_offsets_ptr` deve apontar para
/// `rebase_count` i64s válidos.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_load_snapshot(
    root_arena: i64,
    bytes_ptr: i64,
    bytes_len: i64,
    rebase_offsets_ptr: i64,
    rebase_count: i64,
    snapshot_id: i64,
) {
    if bytes_len <= 0 || bytes_ptr == 0 {
        return;
    }

    // 1. Alocar na root_arena.
    let base_ptr = crate::arena::kata_rt_arena_alloc(root_arena, bytes_len);
    if base_ptr == 0 {
        return;
    }

    // 2. memcpy bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes_ptr as *const u8,
            base_ptr as *mut u8,
            bytes_len as usize,
        );
    }

    // 3. Rebase: para cada offset, somar base_ptr ao i64 naquela posição.
    for i in 0..rebase_count as usize {
        unsafe {
            let offset = std::ptr::read_unaligned(
                (rebase_offsets_ptr as *const u8).add(i * 8) as *const i64
            );
            if offset >= 0 && (offset as usize) + 8 <= bytes_len as usize {
                let fix_ptr = (base_ptr as *mut u8).add(offset as usize) as *mut i64;
                let current = std::ptr::read_unaligned(fix_ptr);
                std::ptr::write_unaligned(fix_ptr, current + base_ptr);
            }
        }
    }

    // 4. Armazenar na tabela TLS.
    SNAPSHOT_PTRS.with(|table| {
        let mut table = table.borrow_mut();
        let id = snapshot_id as usize;
        while table.len() <= id {
            table.push(0);
        }
        table[id] = base_ptr;
    });
}

/// Retorna o ponteiro de um snapshot previamente carregado.
///
/// Retorna 0 se o `snapshot_id` não existe na tabela (não foi carregado).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_get_snapshot(snapshot_id: i64) -> i64 {
    SNAPSHOT_PTRS.with(|table| {
        let table = table.borrow();
        let id = snapshot_id as usize;
        if id < table.len() { table[id] } else { 0 }
    })
}

/// Reseta a tabela de snapshots — chamado entre execuções de teste.
#[allow(dead_code)]
pub(crate) fn reset_snapshot_table() {
    SNAPSHOT_PTRS.with(|table| {
        table.borrow_mut().clear();
    });
}
