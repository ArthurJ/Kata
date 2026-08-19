//! Arena — bump allocator per-fiber + tracked arena para root.
//!
//! Fiber arenas usam bumpalo: alloc O(1), reset O(1), sem dealloc individual.
//! A root arena usa `std::alloc` + tracking: alloc O(1), dealloc O(1) com
//! swap_remove, destroy O(n). Isto permite deallocation individual para
//! valores ARC-managed.
//!
//! Funções C-ABI expostas para o codegen alocar tuplas.
//! Pool de arenas indexado por handle — cada Action cria
//! sua própria arena e a destrói no epílogo. Valores na caller's arena
//! sobrevivem à destruição da arena local.
//!
//! A2 — Runtime reentrante: o pool de arenas e o handle da root arena
//! agora vivem na struct `Runtime` (ver `runtime.rs`). As FFIs recebem
//! `rt: i64` (ponteiro para `*mut Runtime`) como primeiro parâmetro.

use bumpalo::Bump;
use std::alloc::Layout;

use crate::runtime::Runtime;

// ── TLS cache do ponteiro Runtime ativo ─────────────────────────────────
//
// A2 (transitório): As FFIs centrais (scheduler, arena, arc, marshal) recebem
// `rt: i64` explicitamente. As FFIs periféricas (array, list, dict, bytes, etc.)
// leem `rt` deste cache TLS. Isto evita mudar a ABI de ~50 FFIs numa única
// passada. O cache é setado por `kata_rt_scheduler_init` (ou o driver) antes
// de cada execução.
//
// Reentrância: cada execução seta seu próprio `RT_PTR` antes de rodar. REPL
// (sequencial) e LSP (request a request) funcionam. Concorrência real na
// mesma thread não ocorre na prática.
thread_local! {
    static RT_PTR: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

/// Define o ponteiro do Runtime ativo em TLS. Chamado pelo driver/entry point
/// antes de cada execução. As FFIs periféricas leem via `rt_ptr()`.
pub fn set_rt_ptr(rt: i64) {
    RT_PTR.with(|c| c.set(rt));
}

/// Lê o ponteiro do Runtime ativo de TLS. Usado por FFIs periféricas.
pub(crate) fn rt_ptr() -> i64 {
    RT_PTR.with(|c| c.get())
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
    /// a uma alocação anterior. Se `ptr` não está em `blocks`, é no-op.
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

// ── Funções C-ABI para o codegen ─────────────────────────────────────
//
// A2: Todas as FFIs agora recebem `rt: i64` (ponteiro para `*mut Runtime`)
// como primeiro parâmetro. O pool de arenas vive em `Runtime.arenas`.

/// Cria uma nova arena Bump no pool e retorna um handle opaco (índice no Vec).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_create(rt: i64) -> i64 {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    runtime.arena_create()
}

/// Cria uma nova arena Tracked no pool e retorna um handle opaco.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_create_tracked(rt: i64) -> i64 {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    runtime.arena_create_tracked()
}

/// Aloca `size` bytes alinhados a 8 na arena do handle.
/// Retorna o ponteiro para o bloco alocado, ou 0 se falhar.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_alloc(rt: i64, handle: i64, size: i64) -> i64 {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    runtime.arena_alloc(handle, size)
}

/// Libera um bloco individualmente da arena Tracked do handle.
/// No-op para arenas Bump.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_dealloc(rt: i64, handle: i64, ptr: i64, size: i64) {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    runtime.arena_dealloc(handle, ptr, size);
}

/// Reseta SÓ a arena do handle (libera a memória daquela arena).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_destroy(rt: i64, handle: i64) {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    runtime.arena_destroy(handle);
}

/// Retorna (alloc_count, dealloc_count) da arena Tracked do handle.
#[unsafe(no_mangle)]
pub(crate) extern "C" fn kata_rt_arena_stats(rt: i64, handle: i64) -> i64 {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    runtime.arena_stats(handle)
}

/// Lê o handle da root arena do Runtime.
///
/// FFI C-ABI exposta ao codegen — o `alloc_capture_box` chama esta função
/// para obter o handle da root arena onde CaptureBoxes são alocados.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_get_root_arena_handle(rt: i64) -> i64 {
    let runtime = unsafe { &mut *(rt as *mut Runtime) };
    runtime.root_arena_handle
}
