use crate::task;
use crate::memory::{SharedMemory, Arc};

// ============================================================================
// Alocação Local (Arena)
// ============================================================================

#[no_mangle]
pub extern "C" fn kata_rt_alloc_local(size: usize, align: usize) -> *mut u8 {
    task::alloc_local(size, align)
}

#[no_mangle]
pub extern "C" fn kata_rt_clear_local() {
    task::clear_local();
}

// ============================================================================
// Alocação Compartilhada (Heap Global)
// ============================================================================

#[no_mangle]
pub extern "C" fn kata_rt_alloc_shared(size: usize, align: usize) -> *mut u8 {
    SharedMemory::alloc(size, align)
}

#[no_mangle]
pub extern "C" fn kata_rt_free_shared(ptr: *mut u8, size: usize, align: usize) {
    SharedMemory::free(ptr, size, align);
}

// ============================================================================
// ARC - Atomic Reference Counting
// ============================================================================

/// Aloca memória com contagem atômica de referências
/// Retorna ponteiro para os dados (header é interno)
#[no_mangle]
pub extern "C" fn kata_rt_arc_alloc(size: usize, align: usize) -> *mut u8 {
    Arc::alloc(size, align)
}

/// Incrementa o contador de referências ARC
/// Retorna o novo valor do contador
#[no_mangle]
pub extern "C" fn kata_rt_arc_incr(ptr: *mut u8) -> usize {
    Arc::incr(ptr)
}

/// Decrementa o contador de referências ARC
/// Retorna true se a memória foi liberada (última referência)
#[no_mangle]
pub extern "C" fn kata_rt_arc_decr(ptr: *mut u8) -> bool {
    Arc::decr(ptr)
}

/// Obtém o contador de referências atual
#[no_mangle]
pub extern "C" fn kata_rt_arc_count(ptr: *mut u8) -> usize {
    Arc::count(ptr)
}
