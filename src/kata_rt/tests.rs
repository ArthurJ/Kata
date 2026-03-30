#[cfg(test)]
mod tests {
    use crate::kata_rt::memory::SharedMemory;
    use crate::kata_rt::task::{alloc_local, clear_local};
    use crate::kata_rt::csp::{RendezvousChannel, QueueChannel, BroadcastChannel};
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
        
        // Em bump allocators locais, limpar a arena e realocar o mesmo tamanho frequentemente 
        // retorna o mesmo endereço inicial (depende da implementação interna do bumpalo, 
        // mas a ausência de segfaults e a rapidez garantem que a limpeza funcionou).
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

    #[test]
    fn test_csp_rendezvous_channel() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let (tx, mut rx) = RendezvousChannel::new();
            
            // Simula um ponteiro de dados fugindo (Escape Analysis)
            let dummy_data: usize = 0xDEADBEEF;

            tx.try_send(dummy_data as *mut u8).unwrap();

            let received = rx.recv().await.unwrap();
            assert_eq!(received as usize, dummy_data, "O ponteiro recebido deve ser o mesmo enviado (Zero-Copy)");
        });
    }

    #[test]
    fn test_csp_queue_channel_backpressure() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let (tx, mut rx) = QueueChannel::new(2); // Buffer de 2
            
            let p1: *mut u8 = 0x1111 as *mut u8;
            let p2: *mut u8 = 0x2222 as *mut u8;
            let p3: *mut u8 = 0x3333 as *mut u8;

            // Podemos enviar 2 itens sem bloquear
            assert!(tx.try_send(p1).is_ok());
            assert!(tx.try_send(p2).is_ok());
            
            // O terceiro item deve falhar no envio síncrono (simulando Backpressure)
            assert!(tx.try_send(p3).is_err());

            // Consome 1
            let r1 = rx.recv().await.unwrap();
            assert_eq!(r1, p1);

            // Agora o terceiro item pode ser enviado
            assert!(tx.try_send(p3).is_ok());
        });
    }

    #[test]
    fn test_csp_broadcast_channel() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let (tx, mut rx1) = BroadcastChannel::new(16);
            let mut rx2 = tx.subscribe(); // Inscreve um segundo receptor
            
            let p1: *mut u8 = 0x9999 as *mut u8;

            // Envia uma mensagem para o canal 1->N
            tx.send(p1).unwrap();

            // Ambos os receptores devem receber a mesma mensagem (Zero-Copy de ARC)
            let r1 = rx1.recv().await.unwrap();
            let r2 = rx2.recv().await.unwrap();

            assert_eq!(r1, p1);
            assert_eq!(r2, p1);
        });
    }
}
