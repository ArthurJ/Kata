use bumpalo::Bump;
use std::alloc::Layout;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct LocalArena {
    bump: Bump,
}

impl LocalArena {
    pub fn new() -> Self {
        Self { bump: Bump::new() }
    }

    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        let layout = Layout::from_size_align(size, align).unwrap();
        // bumpalo returns NonNull<[u8]>
        self.bump.alloc_layout(layout).as_ptr()
    }

    pub fn clear(&mut self) {
        self.bump.reset();
    }
}

pub struct SharedMemory;

impl SharedMemory {
    pub fn alloc(size: usize, align: usize) -> *mut u8 {
        let layout = Layout::from_size_align(size, align).unwrap();
        unsafe { std::alloc::alloc(layout) }
    }

    pub fn free(ptr: *mut u8, size: usize, align: usize) {
        let layout = Layout::from_size_align(size, align).unwrap();
        unsafe { std::alloc::dealloc(ptr, layout) }
    }
}

// ============================================================================
// ARC - Atomic Reference Counting para dados compartilhados
// ============================================================================

/// Cabeçalho de metadados para alocação ARC
/// Posicionado imediatamente antes do ponteiro retornado ao usuário
#[repr(C)]
struct ArcHeader {
    /// Contador de referências atômico
    ref_count: AtomicUsize,
    /// Tamanho dos dados (sem contar o header)
    size: usize,
    /// Alinhamento dos dados
    align: usize,
}

impl ArcHeader {
    /// Tamanho do header
    const SIZE: usize = std::mem::size_of::<ArcHeader>();
    /// Alinhamento do header
    const ALIGN: usize = std::mem::align_of::<ArcHeader>();
}

/// ARC - Atomic Reference Counting
///
/// Gerencia memória compartilhada com contagem atômica de referências.
/// Dados enviados por canais são automaticamente gerenciados via ARC.
pub struct Arc;

impl Arc {
    /// Aloca memória com ARC
    ///
    /// O contador de referências começa em 1.
    /// Retorna ponteiro para os dados (não para o header).
    pub fn alloc(size: usize, align: usize) -> *mut u8 {
        // Calcula o layout total incluindo o header
        let header_layout = Layout::new::<ArcHeader>();
        let data_layout = Layout::from_size_align(size, align).unwrap();

        // Layout estendido com header
        let (total_layout, _offset) = header_layout
            .extend(data_layout)
            .unwrap();
        let total_layout = total_layout.pad_to_align();

        unsafe {
            let ptr = std::alloc::alloc(total_layout);
            if ptr.is_null() {
                std::alloc::handle_alloc_error(total_layout);
            }

            // Inicializa o header
            let header = ptr as *mut ArcHeader;
            std::ptr::write(header, ArcHeader {
                ref_count: AtomicUsize::new(1),
                size,
                align,
            });

            // Retorna ponteiro para os dados (após o header)
            header.add(1) as *mut u8
        }
    }

    /// Incrementa o contador de referências
    ///
    /// Retorna o novo valor do contador.
    pub fn incr(ptr: *mut u8) -> usize {
        if ptr.is_null() {
            return 0;
        }

        let header = Self::get_header(ptr);
        unsafe {
            header.as_ref().unwrap().ref_count.fetch_add(1, Ordering::Relaxed) + 1
        }
    }

    /// Decrementa o contador de referências
    ///
    /// Retorna true se a memória deve ser liberada (ref_count chegou a 0).
    pub fn decr(ptr: *mut u8) -> bool {
        if ptr.is_null() {
            return false;
        }

        let header = Self::get_header(ptr);
        unsafe {
            let count = header.as_ref().unwrap().ref_count.fetch_sub(1, Ordering::AcqRel);
            if count == 1 {
                // Última referência, liberar memória
                let header_ref = header.as_ref().unwrap();
                let size = header_ref.size;
                let align = header_ref.align;

                let header_layout = Layout::new::<ArcHeader>();
                let data_layout = Layout::from_size_align(size, align).unwrap();
                let (total_layout, _) = header_layout.extend(data_layout).unwrap();
                let total_layout = total_layout.pad_to_align();

                std::alloc::dealloc(header as *mut u8, total_layout);
                true
            } else {
                false
            }
        }
    }

    /// Obtém o contador de referências atual
    pub fn count(ptr: *mut u8) -> usize {
        if ptr.is_null() {
            return 0;
        }

        let header = Self::get_header(ptr);
        unsafe {
            header.as_ref().unwrap().ref_count.load(Ordering::Relaxed)
        }
    }

    /// Obtém o header a partir do ponteiro de dados
    fn get_header(ptr: *mut u8) -> *mut ArcHeader {
        unsafe {
            (ptr as *mut u8).sub(ArcHeader::SIZE) as *mut ArcHeader
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arc_basic() {
        let ptr = Arc::alloc(100, 8);
        assert!(!ptr.is_null());
        assert_eq!(Arc::count(ptr), 1);

        Arc::incr(ptr);
        assert_eq!(Arc::count(ptr), 2);

        Arc::incr(ptr);
        assert_eq!(Arc::count(ptr), 3);

        let freed = Arc::decr(ptr);
        assert!(!freed);
        assert_eq!(Arc::count(ptr), 2);

        Arc::decr(ptr);
        Arc::decr(ptr);
        // Após isso, a memória foi liberada
    }
}
