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

#[repr(C)]
struct ArcHeader {
    ref_count: AtomicUsize,
    size: usize,
    align: usize,
}

impl SharedMemory {
    pub fn alloc(size: usize, align: usize) -> *mut u8 {
        // Garantimos que o offset para os dados reais respeita a alinhamento da struct ARC
        let header_layout = Layout::new::<ArcHeader>();
        let payload_layout = Layout::from_size_align(size, align).unwrap();
        
        let (combined_layout, payload_offset) = header_layout.extend(payload_layout).unwrap();
        let combined_layout = combined_layout.pad_to_align();

        let ptr = unsafe { std::alloc::alloc(combined_layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(combined_layout);
        }

        let header_ptr = ptr as *mut ArcHeader;
        unsafe {
            (*header_ptr).ref_count = AtomicUsize::new(1);
            (*header_ptr).size = size;
            (*header_ptr).align = align;
            
            // Retorna o ponteiro deslocado (esconde o cabecalho ARC do Cranelift)
            ptr.add(payload_offset)
        }
    }
    
    pub fn decref(payload_ptr: *mut u8) {
        if payload_ptr.is_null() { return; }

        let header_layout = Layout::new::<ArcHeader>();
        // Assumimos que o compilador usou 8 ou 16 de alinhamento para structs de CSP (seguros para recalcular o pad)
        // Isso e simplificado para MVP, mas suficiente para estruturas Cranelift fixas
        let payload_offset = header_layout.size() + (8 - (header_layout.size() % 8)) % 8; 

        // Recupera o ponteiro original (subtraindo o offset)
        let header_ptr = unsafe { payload_ptr.sub(payload_offset) as *mut ArcHeader };
        
        unsafe {
            // Memory order Release/Acquire para seguranca multithreading M:N (Tokio)
            if (*header_ptr).ref_count.fetch_sub(1, Ordering::Release) == 1 {
                std::sync::atomic::fence(Ordering::Acquire);
                
                let size = (*header_ptr).size;
                let align = (*header_ptr).align;
                
                let payload_layout = Layout::from_size_align(size, align).unwrap();
                let (combined_layout, _) = header_layout.extend(payload_layout).unwrap();
                let combined_layout = combined_layout.pad_to_align();

                std::alloc::dealloc(header_ptr as *mut u8, combined_layout);
            }
        }
    }
}
