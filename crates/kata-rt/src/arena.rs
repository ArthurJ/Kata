//! Arena — bump allocator para todas as arenas (fiber + root).
//!
//! Todas as arenas usam bumpalo: alloc O(1), reset O(1), sem dealloc
//! individual. A root arena nunca é resetada entre fibers (só destruída
//! no `Drop` do Runtime). Fiber arenas são resetadas quando o fiber
//! termina.
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

use crate::runtime::deref_runtime;

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

// ── ArenaKind: enum dispatch no pool ──────────────────────────────────

/// Tipo de arena no pool. Todas as arenas são Bump (bumpalo).
pub(crate) enum ArenaKind {
    /// Arena Bump — fast path, sem dealloc individual.
    Bump(Arena),
}

// ── Funções C-ABI para o codegen ─────────────────────────────────────
//
// A2: Todas as FFIs agora recebem `rt: i64` (ponteiro para `*mut Runtime`)
// como primeiro parâmetro. O pool de arenas vive em `Runtime.arenas`.

/// Cria uma nova arena Bump no pool e retorna um handle opaco (índice no Vec).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_create(rt: i64) -> i64 {
    let runtime = unsafe { deref_runtime(rt) };
    runtime.arena_create()
}

/// Aloca `size` bytes alinhados a 8 na arena do handle.
/// Retorna o ponteiro para o bloco alocado, ou 0 se falhar.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_alloc(rt: i64, handle: i64, size: i64) -> i64 {
    let runtime = unsafe { deref_runtime(rt) };
    runtime.arena_alloc(handle, size)
}

/// Reseta SÓ a arena do handle (libera a memória daquela arena).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_arena_destroy(rt: i64, handle: i64) {
    let runtime = unsafe { deref_runtime(rt) };
    runtime.arena_destroy(handle);
}

/// Lê o handle da root arena do Runtime.
///
/// FFI C-ABI exposta ao codegen — usada por `escape_arena` e `action_call`
/// para obter o handle da root arena (agora Bump).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_get_root_arena_handle(rt: i64) -> i64 {
    let runtime = unsafe { deref_runtime(rt) };
    runtime.root_arena_handle
}
