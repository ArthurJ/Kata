use super::memory::LocalArena;
use std::cell::RefCell;

thread_local! {
    pub static LOCAL_ARENA: RefCell<LocalArena> = RefCell::new(LocalArena::new());
}

pub fn alloc_local(size: usize, align: usize) -> *mut u8 {
    LOCAL_ARENA.with(|arena| {
        arena.borrow_mut().alloc(size, align)
    })
}

pub fn clear_local() {
    LOCAL_ARENA.with(|arena| {
        arena.borrow_mut().clear();
    })
}
