//! Arena — bump allocator per-fiber.
//!
//! Em Fio 1: estrutura pronta, mas sem uso (Actions vêm em Fio 3).
//! A arena libera tudo em O(1) no epílogo da Action.
//!
//! Em Fio 2 (DoD 22): funções C-ABI expostas para o codegen alocar tuplas.
//! Em Fio 3 (Fase 3): pool de arenas indexado por handle — cada Action cria
//! sua própria arena e a destrói no epílogo. Valores na caller's arena
//! sobrevivem à destruição da arena local.
//!
//! `kata_rt_arena_create` cria uma nova arena no pool e retorna um handle
//! opaco (índice no Vec). `kata_rt_arena_alloc(handle, size)` aloca `size`
//! bytes alinhados a 8 na arena do handle. `kata_rt_arena_destroy(handle)`
//! reseta SÓ a arena do handle (não o pool inteiro).

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

    /// Aloca `size` bytes alinhado a `align`. Retorna ponteiro bruto.
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

// ── Pool de arenas thread-local para FFI ─────────────────────────────

thread_local! {
    static ARENAS: RefCell<Vec<Arena>> = RefCell::new(Vec::new());
}

/// Reseta todas as arenas do pool thread-local. Chamado entre execuções
/// de teste para evitar poluição de estado global.
pub fn reset_all_arenas() {
    ARENAS.with(|arenas| {
        arenas.borrow_mut().clear();
    });
}

// ── Funções C-ABI para o codegen ─────────────────────────────────────

/// Cria uma nova arena no pool e retorna um handle opaco (índice no Vec).
///
/// O handle é válido até `kata_rt_arena_destroy(handle)` ser chamado.
/// Handles não são reusados — cada `arena_create` produz um handle novo.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_create() -> i64 {
    ARENAS.with(|arenas| {
        let mut arenas = arenas.borrow_mut();
        let id = arenas.len() as i64;
        arenas.push(Arena::new());
        id
    })
}

/// Aloca `size` bytes alinhados a 8 na arena do handle.
/// Retorna o ponteiro para o bloco alocado, ou 0 se falhar.
///
/// # Safety
/// `handle` deve ser um valor retornado por `kata_rt_arena_create`.
/// `size` deve ser > 0.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_alloc(handle: i64, size: i64) -> i64 {
    if size <= 0 {
        return 0;
    }
    let layout = match std::alloc::Layout::from_size_align(size as usize, 8) {
        Ok(l) => l,
        Err(_) => return 0,
    };
    ARENAS.with(|arenas| {
        let arenas = arenas.borrow();
        let idx = handle as usize;
        if idx >= arenas.len() {
            return 0;
        }
        let ptr = arenas[idx].alloc(layout);
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

/// Reseta SÓ a arena do handle (libera a memória daquela arena).
/// Outras arenas no pool não são afetadas.
///
/// # Safety
/// `handle` deve ser um valor retornado por `kata_rt_arena_create`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_destroy(handle: i64) {
    if handle < 0 {
        return;
    }
    ARENAS.with(|arenas| {
        let mut arenas = arenas.borrow_mut();
        let idx = handle as usize;
        if let Some(a) = arenas.get_mut(idx) {
            a.reset();
        }
    })
}
