use bumpalo::Bump;
use std::alloc::Layout;

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
