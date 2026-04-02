#[cfg(test)]
mod tests {
    use crate::kata_rt::memory::SharedMemory;
    use crate::kata_rt::task::{alloc_local, clear_local};
    use tokio::runtime::Runtime;

    #[test]
    fn test_local_arena_allocation_and_clear() {
        // Aloca 100 bytes na Arena Local da thread atual
        let ptr1 = alloc_local(100, 8);
        assert!(!ptr1.is_null());

        let ptr2 = alloc_local(50, 8);
        assert!(!ptr2.is_null());
        
        assert_ne!(ptr1, ptr2, "Ponteiros devem ser diferentes");

        // Limpa a arena inteira (simulando o fim de uma Action no Tokio)
        clear_local();

        // Após o clear, uma nova alocação deve reaproveitar a memória do início da Arena
        let ptr3 = alloc_local(100, 8);
        assert!(!ptr3.is_null());
    }

    #[test]
    fn test_shared_memory_allocation() {
        let size = 1024;
        let align = 8;
        let ptr = SharedMemory::alloc(size, align);
        assert!(!ptr.is_null());
        
        // Escreve algo para testar se a memória é válida
        unsafe {
            *ptr = 42;
            assert_eq!(*ptr, 42);
        }

        // Libera a memória compartilhada
        SharedMemory::free(ptr, size, align);
    }
}
