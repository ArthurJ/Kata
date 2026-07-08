//! Arena — bump allocator per-fiber.
//!
//! Em Fio 1: estrutura pronta, mas sem uso (Actions vêm em Fio 3).
//! A arena libera tudo em O(1) no epílogo da Action.

use bumpalo::Bump;

/// Arena per-fiber. Dados locais são alocados aqui e liberados em O(1).
pub struct Arena {
    bump: Bump,
}

impl Arena {
    pub fn new() -> Self {
        Arena { bump: Bump::new() }
    }

    /// Aloca `size` bytes alinhados a `align`. Retorna ponteiro bruto.
    pub fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        self.bump.alloc_layout(layout).as_ptr() as *mut u8
    }

    /// Reseta a arena (libera tudo). O(1).
    pub fn reset(&mut self) {
        self.bump = Bump::new();
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}
