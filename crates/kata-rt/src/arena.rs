//! Arena — bump allocator per-fiber.
//!
//! Em Fio 1: estrutura pronta, mas sem uso (Actions vêm em Fio 3).
//! A arena libera tudo em O(1) no epílogo da Action.
//!
//! Em Fio 2 (DoD 22): funções C-ABI expostas para o codegen alocar tuplas.
//! A arena é thread-local — cada thread (fiber, no futuro) tem sua própria.
//! `kata_rt_arena_create` inicializa a arena thread-local e retorna um handle
//! opaco (ponteiro para a `Arena`). `kata_rt_arena_alloc(handle, size)` aloca
//! `size` bytes alinhados a 8 e retorna o ponteiro. `kata_rt_arena_destroy`
//! reseta a arena.

use bumpalo::Bump;
use std::cell::RefCell;

/// Arena per-fiber. Dados locais são alocados aqui e liberados em O(1).
pub(crate) struct Arena {
    bump: Bump,
}

impl Arena {
    pub(crate) fn new() -> Self {
        Arena { bump: Bump::new() }
    }

    /// Aloca `size` bytes alinhados a `align`. Retorna ponteiro bruto.
    pub(crate) fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        self.bump.alloc_layout(layout).as_ptr()
    }

    /// Reseta a arena (libera tudo). O(1).
    pub(crate) fn reset(&mut self) {
        self.bump = Bump::new();
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

// ── Arena thread-local para FFI ──────────────────────────────────────

thread_local! {
    static ARENA: RefCell<Arena> = RefCell::new(Arena::new());
}

/// Reseta a arena thread-local. Chamado entre execuções de teste para
/// evitar poluição de estado global.
pub fn reset_arena() {
    ARENA.with(|a| a.borrow_mut().reset());
}

// ── Funções C-ABI para o codegen ─────────────────────────────────────

/// Cria (ou obtém) a arena thread-local e retorna um handle opaco.
/// O handle é um ponteiro para a `Arena` interna — usado por `arena_alloc`.
///
/// Em Fio 2, chamado uma vez no início do `__kata_entry`. Quando Fio 3
/// adicionar fibers, cada fiber terá sua própria arena thread-local.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_create() -> i64 {
    // Retorna um handle não-nulo. Como a arena é thread_local, o handle
    // é apenas um sentinel — `arena_alloc` usa a thread_local diretamente.
    // Usamos 1 como sentinel válido (0 seria "arena não inicializada").
    1
}

/// Aloca `size` bytes alinhados a 8 na arena thread-local.
/// Retorna o ponteiro para o bloco alocado, ou 0 se falhar.
///
/// # Safety
/// `handle` deve ser um valor retornado por `kata_rt_arena_create`.
/// `size` deve ser > 0.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_alloc(_handle: i64, size: i64) -> i64 {
    if size <= 0 {
        return 0;
    }
    let layout = match std::alloc::Layout::from_size_align(size as usize, 8) {
        Ok(l) => l,
        Err(_) => return 0,
    };
    ARENA.with(|a| {
        let ptr = a.borrow().alloc(layout);
        if ptr.is_null() {
            0
        } else {
            // Zera o bloco para evitar garbage de alocações anteriores.
            unsafe {
                std::ptr::write_bytes(ptr, 0, size as usize);
            }
            ptr as i64
        }
    })
}

/// Reseta a arena thread-local (libera toda a memória alocada).
/// Chamado no final do `__kata_entry` ou entre execuções.
///
/// # Safety
/// `handle` deve ser um valor retornado por `kata_rt_arena_create`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_destroy(handle: i64) {
    if handle != 0 {
        reset_arena();
    }
}
