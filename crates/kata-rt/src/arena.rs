//! Arena — bump allocator per-fiber + tracked arena para root.
//!
//! Fiber arenas usam bumpalo: alloc O(1), reset O(1), sem dealloc individual.
//! A root arena usa `std::alloc` + tracking: alloc O(1), dealloc O(1) com
//! swap_remove, destroy O(n). Isto permite deallocation individual para
//! valores ARC-managed (Fio 16).
//!
//! Funções C-ABI expostas para o codegen alocar tuplas.
//! Pool de arenas indexado por handle — cada Action cria
//! sua própria arena e a destrói no epílogo. Valores na caller's arena
//! sobrevivem à destruição da arena local.
//!
//! `kata_rt_arena_create` cria uma nova arena Bump no pool e retorna um handle
//! opaco (índice no Vec). `kata_rt_arena_create_tracked` cria uma arena Tracked.
//! `kata_rt_arena_alloc(handle, size)` aloca `size` bytes alinhados a 8 na
//! arena do handle. `kata_rt_arena_destroy(handle)` reseta SÓ a arena do
//! handle (não o pool inteiro).
//!
//! TLS `ROOT_ARENA_HANDLE` — setada por `kata_rt_scheduler_init`, lida pelo
//! codegen e por `kata_rt_decref` (Fio 16) para acessar a root arena.

use bumpalo::Bump;
use std::alloc::Layout;
use std::cell::{Cell, RefCell};

// ── TLS: handle da root arena ─────────────────────────────────────────
//
// Setada por `kata_rt_scheduler_init` (ffi.rs). Lida pelo codegen
// (LowerCtx.root_arena) e por `kata_rt_decref` (arc.rs) para liberar
// blocos ARC-managed individualmente.

thread_local! {
    pub(crate) static ROOT_ARENA_HANDLE: Cell<i64> = const { Cell::new(0) };
}

/// Define o handle da root arena em TLS. Chamado por `kata_rt_scheduler_init`.
pub(crate) fn set_root_arena_handle(handle: i64) {
    ROOT_ARENA_HANDLE.with(|h| h.set(handle));
}

/// Lê o handle da root arena de TLS. Retorna 0 se não inicializado.
///
/// FFI C-ABI exposta ao codegen — o `alloc_capture_box` chama esta função
/// para obter o handle da root arena onde CaptureBoxes são alocados (Fio 16).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_get_root_arena_handle() -> i64 {
    ROOT_ARENA_HANDLE.with(|h| h.get())
}

// ── Fiber arena (bumpalo) ─────────────────────────────────────────────

/// Arena per-fiber. Dados locais são alocados aqui e liberados em O(1).
pub(crate) struct Arena {
    bump: Bump,
}

impl Arena {
    pub(crate) fn new() -> Self {
        Arena { bump: Bump::new() }
    }

    /// Aloca `size` bytes alinhado a `align`. Retorna ponteiro bruto.
    pub(crate) fn alloc(&self, layout: Layout) -> *mut u8 {
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

// ── Root arena (std::alloc + tracking) ────────────────────────────────

/// Arena tracked — `std::alloc` + tracking para dealloc individual.
///
/// Usada pela root arena para valores ARC-managed que precisam sobreviver
/// à destruição de fibers individuais e ser liberados individualmente
/// quando o refcount chega a 0.
///
/// - `alloc`: `std::alloc::alloc(layout)` + `blocks.push((ptr, layout))` [O(1)+push]
/// - `dealloc`: encontra ptr em `blocks`, `swap_remove`, `std::alloc::dealloc` [O(n) busca, O(1) remove]
/// - `destroy`: percorre `blocks`, `dealloc` cada um [O(n)]
pub(crate) struct TrackedArena {
    /// Blocos alocados e ainda vivos. Usado para dealloc individual e teardown.
    blocks: Vec<(*mut u8, Layout)>,
    /// Contador de alocações (para testes de leak counting).
    pub(crate) alloc_count: u64,
    /// Contador de dealocações individuais (para testes de leak counting).
    /// Não inclui `destroy()` (bulk dealloc da arena inteira).
    pub(crate) dealloc_count: u64,
}

impl TrackedArena {
    pub(crate) fn new() -> Self {
        TrackedArena {
            blocks: Vec::new(),
            alloc_count: 0,
            dealloc_count: 0,
        }
    }

    /// Aloca `layout` bytes via `std::alloc`. Retorna ponteiro bruto.
    /// Rastreia o bloco para dealloc individual e teardown.
    pub(crate) fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { std::alloc::alloc(layout) };
        if !ptr.is_null() {
            self.blocks.push((ptr, layout));
            self.alloc_count += 1;
        }
        ptr
    }

    /// Libera um bloco individualmente. `ptr` e `layout` devem corresponder
    /// a uma alocação anterior. Se `ptr` não está em `blocks`, é no-op
    /// (defensivo — evita panic em double-dealloc).
    pub(crate) fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        if let Some(idx) = self.blocks.iter().position(|(p, _)| *p == ptr) {
            self.blocks.swap_remove(idx);
            unsafe { std::alloc::dealloc(ptr, layout) };
            self.dealloc_count += 1;
        }
    }

    /// Libera todos os blocos restantes. Chamado no teardown da root arena.
    pub(crate) fn destroy(&mut self) {
        for (ptr, layout) in self.blocks.drain(..) {
            unsafe { std::alloc::dealloc(ptr, layout) };
        }
    }
}

impl Default for TrackedArena {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TrackedArena {
    fn drop(&mut self) {
        // Safety: todos os blocos em `self.blocks` foram alocados com
        // `std::alloc::alloc` com o Layout registrado. Liberar no drop
        // evita leak se a TrackedArena for dropada sem `destroy()` explícito.
        self.destroy();
    }
}

// ── ArenaKind: enum dispatch no pool ──────────────────────────────────

/// Tipo de arena no pool. Dispatch por enum — sem trait object overhead.
pub(crate) enum ArenaKind {
    /// Fiber arena — bumpalo, fast path, sem dealloc individual.
    Bump(Arena),
    /// Root arena — std::alloc + tracking, dealloc individual.
    Tracked(TrackedArena),
}

// ── Pool de arenas thread-local para FFI ─────────────────────────────

thread_local! {
    static ARENAS: RefCell<Vec<ArenaKind>> = const { RefCell::new(Vec::new()) };
}

/// Reseta todas as arenas do pool thread-local. Chamado entre execuções
/// de teste para evitar poluição de estado global.
#[allow(dead_code)]
pub(crate) fn reset_all_arenas() {
    ARENAS.with(|arenas| {
        arenas.borrow_mut().clear();
    });
    ROOT_ARENA_HANDLE.with(|h| h.set(0));
}

// ── Funções C-ABI para o codegen ─────────────────────────────────────

/// Cria uma nova arena Bump no pool e retorna um handle opaco (índice no Vec).
///
/// O handle é válido até `kata_rt_arena_destroy(handle)` ser chamado.
/// Handles não são reusados — cada `arena_create` produz um handle novo.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_create() -> i64 {
    ARENAS.with(|arenas| {
        let mut arenas = arenas.borrow_mut();
        let id = arenas.len() as i64;
        arenas.push(ArenaKind::Bump(Arena::new()));
        id
    })
}

/// Cria uma nova arena Tracked no pool e retorna um handle opaco.
///
/// Arena Tracked usa `std::alloc` + tracking para dealloc individual.
/// Usada para a root arena (valores ARC-managed).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_create_tracked() -> i64 {
    ARENAS.with(|arenas| {
        let mut arenas = arenas.borrow_mut();
        let id = arenas.len() as i64;
        arenas.push(ArenaKind::Tracked(TrackedArena::new()));
        id
    })
}

/// Aloca `size` bytes alinhados a 8 na arena do handle.
/// Retorna o ponteiro para o bloco alocado, ou 0 se falhar.
///
/// Dispatcha por tipo de arena:
/// - `Bump` → `bump.alloc_layout(layout)` (inalterado)
/// - `Tracked` → `std::alloc::alloc(layout)` + tracking
///
/// # Safety
/// `handle` deve ser um valor retornado por `kata_rt_arena_create` ou
/// `kata_rt_arena_create_tracked`. `size` deve ser > 0.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_alloc(handle: i64, size: i64) -> i64 {
    if size <= 0 {
        return 0;
    }
    let layout = match Layout::from_size_align(size as usize, 8) {
        Ok(l) => l,
        Err(_) => return 0,
    };
    ARENAS.with(|arenas| {
        let mut arenas = arenas.borrow_mut();
        let idx = handle as usize;
        if idx >= arenas.len() {
            return 0;
        }
        let ptr = match &mut arenas[idx] {
            ArenaKind::Bump(a) => a.alloc(layout),
            ArenaKind::Tracked(t) => t.alloc(layout),
        };
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

/// Libera um bloco individualmente da arena Tracked do handle.
///
/// Para arenas Bump, é no-op (bumpalo não suporta dealloc individual).
/// Para arenas Tracked, chama `std::alloc::dealloc` e remove do tracking.
///
/// `size` deve corresponder ao `size` passado em `kata_rt_arena_alloc`.
///
/// # Safety
/// `handle` deve ser válido. `ptr` deve ser um ponteiro retornado por
/// `kata_rt_arena_alloc` na arena do handle. `size` deve ser o mesmo
/// usado na alocação.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_dealloc(handle: i64, ptr: i64, size: i64) {
    if handle < 0 || ptr == 0 || size <= 0 {
        return;
    }
    let layout = match Layout::from_size_align(size as usize, 8) {
        Ok(l) => l,
        Err(_) => return,
    };
    ARENAS.with(|arenas| {
        let mut arenas = arenas.borrow_mut();
        let idx = handle as usize;
        if idx >= arenas.len() {
            return;
        }
        if let ArenaKind::Tracked(t) = &mut arenas[idx] {
            t.dealloc(ptr as *mut u8, layout);
        }
        // Bump: no-op — bumpalo não suporta dealloc individual.
    })
}

/// Reseta SÓ a arena do handle (libera a memória daquela arena).
/// Outras arenas no pool não são afetadas.
///
/// Para arenas Bump: `bump.reset()` (O(1), libera tudo).
/// Para arenas Tracked: percorre `blocks` e `dealloc` cada um (O(n)).
///
/// # Safety
/// `handle` deve ser um valor retornado por `kata_rt_arena_create` ou
/// `kata_rt_arena_create_tracked`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_destroy(handle: i64) {
    if handle < 0 {
        return;
    }
    ARENAS.with(|arenas| {
        let mut arenas = arenas.borrow_mut();
        let idx = handle as usize;
        if let Some(a) = arenas.get_mut(idx) {
            match a {
                ArenaKind::Bump(b) => b.reset(),
                ArenaKind::Tracked(t) => t.destroy(),
            }
        }
    })
}

/// Retorna (alloc_count, dealloc_count) da arena Tracked do handle.
///
/// Para arenas Bump, retorna (0, 0) (não rastreia individualmente).
/// Usado por testes de leak counting para verificar que allocs e deallocs
/// individuais fecham no fim da execução.
///
/// `dealloc_count` inclui só dealocações individuais (`kata_rt_arena_dealloc`),
/// não `arena_destroy` (bulk). Após `destroy()`, ambos voltam a 0.
///
/// # Safety
/// `handle` deve ser válido.
#[unsafe(no_mangle)]
pub(crate) extern "C" fn kata_rt_arena_stats(handle: i64) -> i64 {
    if handle < 0 {
        return 0;
    }
    ARENAS.with(|arenas| {
        let arenas = arenas.borrow();
        let idx = handle as usize;
        if idx >= arenas.len() {
            return 0;
        }
        match &arenas[idx] {
            ArenaKind::Tracked(t) => {
                // Codifica os dois contadores em um único i64:
                // bits 0-31: alloc_count, bits 32-63: dealloc_count
                ((t.dealloc_count as i64) << 32) | (t.alloc_count as i64 & 0xFFFF_FFFF)
            }
            _ => 0,
        }
    })
}
