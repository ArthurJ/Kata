//! List — Cons cells alocadas na arena.
//!
//! Layout do Cons cell (16 bytes):
//! ```text
//! offset 0: head (i64) — valor do elemento
//! offset 8: tail (i64) — ponteiro para o próximo Cons ou 0 (Nil)
//! ```
//!
//! Nil = null pointer (0). `kata_rt_list_nil` retorna 0.

/// Retorna 0 (null = Nil). Existe para simetria com o FfiSymbol.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_nil() -> i64 {
    0
}

/// Aloca um Cons cell (16 bytes) na arena especificada.
/// Store head no offset 0, tail no offset 8. Retorna ponteiro.
///
/// # Safety
/// `head` e `tail` são valores i64 válidos. `arena_handle` é um handle válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_cons(head: i64, tail: i64, arena_handle: i64) -> i64 {
    let ptr = crate::arena::kata_rt_arena_alloc(arena_handle, 16);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        std::ptr::write_unaligned(ptr as *mut i64, head);
        std::ptr::write_unaligned((ptr as *mut u8).add(8) as *mut i64, tail);
    }
    ptr
}

/// Retorna 1 se a lista é vazia (ptr == 0), 0 caso contrário.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_is_empty(ptr: i64) -> i64 {
    if ptr == 0 { 1 } else { 0 }
}

/// Extrai o head de um Cons cell (load offset 0).
///
/// # Safety
/// `ptr` deve ser um ponteiro válido para Cons cell (não-Nil).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_head(ptr: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    unsafe { std::ptr::read_unaligned(ptr as *const i64) }
}

/// Extrai o tail de um Cons cell (load offset 8).
///
/// # Safety
/// `ptr` deve ser um ponteiro válido para Cons cell (não-Nil).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_tail(ptr: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    unsafe { std::ptr::read_unaligned((ptr as *const u8).add(8) as *const i64) }
}

/// Conta o número de Cons cells (O(n)). Retorna SMI-tagged.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_len(ptr: i64) -> i64 {
    let mut count = 0i64;
    let mut current = ptr;
    while current != 0 {
        count += 1;
        current = unsafe { std::ptr::read_unaligned((current as *const u8).add(8) as *const i64) };
    }
    // SMI tag: (val << 1) | 1
    (count << 1) | 1
}

/// Acesso por índice com bounds check. Retorna um Result box (Sum):
/// - Ok: tag=0, payload=valor
/// - Err: tag=1, payload=0 (out of bounds)
///
/// Layout do Result box (igual a store_sum_result): 16 bytes, tag no offset 0, payload no offset 8.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_get_checked(ptr: i64, idx: i64) -> i64 {
    let mut current = ptr;
    let mut i = 0i64;
    while current != 0 && i < idx {
        i += 1;
        current = unsafe { std::ptr::read_unaligned((current as *const u8).add(8) as *const i64) };
    }
    if current == 0 || i < idx {
        // Out of bounds — retorna Err (tag=1, payload=0)
        return crate::sum::kata_rt_store_sum_result(1, 0, 0);
    }
    // Found — retorna Ok (tag=0, payload=head)
    let head = unsafe { std::ptr::read_unaligned(current as *const i64) };
    crate::sum::kata_rt_store_sum_result(0, head, 0)
}

/// Verifica se `item` está na lista (percorre Cons cells). Retorna 1 (true) ou 0 (false).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_contains(ptr: i64, item: i64) -> i64 {
    let mut current = ptr;
    while current != 0 {
        let head = unsafe { std::ptr::read_unaligned(current as *const i64) };
        if head == item {
            return 1;
        }
        current = unsafe { std::ptr::read_unaligned((current as *const u8).add(8) as *const i64) };
    }
    0
}

/// Reverte uma lista Cons (O(n), aloca nova lista na arena).
///
/// Percorre a lista original de frente, faz `cons(head, acc)` para cada
/// elemento — o resultado é a lista reversa. Usado por map/filter no
/// codegen: constroem a lista com prepend (que inverte) e chamam reverse
/// para restaurar a ordem original.
///
/// # Safety
/// `ptr` é um ponteiro para Cons cell ou 0 (Nil). `arena_handle` é válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_reverse(ptr: i64, arena_handle: i64) -> i64 {
    let mut acc: i64 = 0; // Nil
    let mut current = ptr;
    while current != 0 {
        let head = unsafe { std::ptr::read_unaligned(current as *const i64) };
        acc = kata_rt_list_cons(head, acc, arena_handle);
        current = unsafe { std::ptr::read_unaligned((current as *const u8).add(8) as *const i64) };
    }
    acc
}

/// Concatena duas listas Cons (O(n) onde n = len da primeira lista).
///
/// Percorre a primeira lista, faz `cons(head, acc)` para cada elemento
/// (invertendo), depois reverte o acc e faz `cons(head, second)` para
/// cada elemento do acc (invertendo de volta) — resultado preserva
/// a ordem de ambas as listas.
///
/// Alternativa mais simples: percorre a primeira lista recursivamente
/// fazendo cons no final. Mas iteração é mais segura que recursão
/// (sem risco de stack overflow para listas grandes).
///
/// # Safety
/// `first` e `second` são ponteiros para Cons cell ou 0 (Nil). `arena_handle` é válido.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_list_concat(first: i64, second: i64, arena_handle: i64) -> i64 {
    // Strategy: reverse first, then cons each element of reversed-first onto second.
    // reverse(first) gives us first's elements in reverse order.
    // consing them onto second prepends them in reverse-reverse = original order.
    let reversed = kata_rt_list_reverse(first, arena_handle);
    let mut current = reversed;
    let mut acc = second;
    while current != 0 {
        let head = unsafe { std::ptr::read_unaligned(current as *const i64) };
        acc = kata_rt_list_cons(head, acc, arena_handle);
        current = unsafe { std::ptr::read_unaligned((current as *const u8).add(8) as *const i64) };
    }
    acc
}
