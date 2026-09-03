//! Runtime reentrante — estado explícito em vez de TLS.
//!
//! A struct `Runtime` carrega todo o estado que antes vivia em `thread_local!`:
//! scheduler, pool de arenas, type table, e contadores de yield. Passada por
//! ponteiro (`*mut Runtime` como `i64`) a todas as FFIs que precisam de estado
//! de execução. Torna o runtime reentrante — múltiplas execuções isoladas no
//! mesmo processo (REPL, LSP).
//!
//! O que **não** vive aqui (permanece TLS ou global):
//! - `CURRENT_SUSPEND` / `LAST_SUSPEND_PTR` — ponteiros de continuação do fiber,
//!   válidos apenas durante `resume()`, limpos pelo `SuspendGuard::drop`.
//! - `TIMEOUT_EXPIRED` / `PENDING_TIMER` — timer de teste; a thread OS timer
//!   não tem acesso ao `Runtime*`.
//! - Log, snapshot, dict, cache — TLS periféricas, migram depois.

use std::cell::Cell;

use crate::arena::{Arena, ArenaKind, TrackedArena};
use crate::marshal::TypeShape;
use crate::scheduler::Scheduler;

/// Default recursion depth limit for both backends.
pub const DEFAULT_DEPTH_LIMIT: u32 = 1000;

/// Runtime explícito — substitui TLS do scheduler, arenas, type table e yield.
///
/// Alocado pelo driver/REPL/LSP e passado por ponteiro (`*mut Runtime` como
/// `i64`) ao código JIT. Cada execução isolada cria seu próprio `Runtime`.
pub struct Runtime {
    /// Scheduler de fibers. Antes em `SCHEDULER: RefCell<Option<Scheduler>>` TLS.
    pub(crate) scheduler: Scheduler,
    /// Pool de arenas (Bump + Tracked). Antes em `ARENAS: RefCell<Vec<ArenaKind>>` TLS.
    pub(crate) arenas: Vec<ArenaKind>,
    /// Handle (índice em `arenas`) da root arena (Tracked). Antes em
    /// `ROOT_ARENA_HANDLE: Cell<i64>` TLS.
    pub root_arena_handle: i64,
    /// Type shapes para marshalling. Antes em `TYPE_TABLE: RefCell<Vec<TypeShape>>` TLS.
    pub type_table: Vec<TypeShape>,
    /// Contador de profundidade de recursão (incrementado no prólogo de cada
    /// chamada não-de-cauda, decrementado no epílogo). `Cell` porque o Runtime
    /// é single-threaded com scheduler cooperativo.
    call_depth: Cell<u32>,
    /// Limite de profundidade de recursão. Configurável via
    /// `set_recursion_limit` em `stdlib/config.kata`. Default 1000.
    depth_limit: Cell<u32>,
    /// Flag de overflow — setada quando `call_depth` excede `depth_limit`.
    /// O entry point checa após o retorno, em vez de comparar valor sentinela.
    overflowed: Cell<bool>,
}

impl Runtime {
    /// Cria um novo Runtime com scheduler vazio e root arena (Tracked) alocada.
    ///
    /// A root arena é criada diretamente via `TrackedArena::new()` — não chama
    /// a FFI `kata_rt_arena_create_tracked` (que precisaria de `rt` que ainda
    /// não existe durante a construção).
    pub fn new() -> Self {
        let arenas: Vec<ArenaKind> = vec![ArenaKind::Tracked(TrackedArena::new())];
        let root_arena_handle = 0; // índice da primeira arena no pool

        // Scheduler::new() não cria mais a root arena — recebe o handle.
        let scheduler = Scheduler::new(root_arena_handle);

        Runtime {
            scheduler,
            arenas,
            root_arena_handle,
            type_table: Vec::new(),
            call_depth: Cell::new(0),
            depth_limit: Cell::new(DEFAULT_DEPTH_LIMIT),
            overflowed: Cell::new(false),
        }
    }

    /// Cria uma arena Bump no pool e retorna o handle (índice).
    pub fn arena_create(&mut self) -> i64 {
        let id = self.arenas.len() as i64;
        self.arenas.push(ArenaKind::Bump(Arena::new()));
        id
    }

    /// Cria uma arena Tracked no pool e retorna o handle (índice).
    pub fn arena_create_tracked(&mut self) -> i64 {
        let id = self.arenas.len() as i64;
        self.arenas.push(ArenaKind::Tracked(TrackedArena::new()));
        id
    }

    /// Aloca `size` bytes (align 8) na arena do handle. Retorna ptr ou 0.
    pub fn arena_alloc(&mut self, handle: i64, size: i64) -> i64 {
        if size <= 0 {
            return 0;
        }
        let layout = match std::alloc::Layout::from_size_align(size as usize, 8) {
            Ok(l) => l,
            Err(_) => return 0,
        };
        let idx = handle as usize;
        if idx >= self.arenas.len() {
            return 0;
        }
        let ptr = match &mut self.arenas[idx] {
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
    }

    /// Libera um bloco individualmente da arena Tracked do handle.
    /// No-op para arenas Bump (bumpalo não suporta dealloc individual).
    pub fn arena_dealloc(&mut self, handle: i64, ptr: i64, size: i64) {
        if handle < 0 || ptr == 0 || size <= 0 {
            return;
        }
        let layout = match std::alloc::Layout::from_size_align(size as usize, 8) {
            Ok(l) => l,
            Err(_) => return,
        };
        let idx = handle as usize;
        if idx >= self.arenas.len() {
            return;
        }
        if let ArenaKind::Tracked(t) = &mut self.arenas[idx] {
            t.dealloc(ptr as *mut u8, layout);
        }
    }

    /// Reseta SÓ a arena do handle (libera a memória daquela arena).
    pub fn arena_destroy(&mut self, handle: i64) {
        if handle < 0 {
            return;
        }
        let idx = handle as usize;
        if let Some(a) = self.arenas.get_mut(idx) {
            match a {
                ArenaKind::Bump(b) => b.reset(),
                ArenaKind::Tracked(t) => t.destroy(),
            }
        }
    }

    /// Retorna (alloc_count, dealloc_count) da arena Tracked do handle.
    pub fn arena_stats(&self, handle: i64) -> i64 {
        if handle < 0 {
            return 0;
        }
        let idx = handle as usize;
        if idx >= self.arenas.len() {
            return 0;
        }
        match &self.arenas[idx] {
            ArenaKind::Tracked(t) => {
                ((t.dealloc_count as i64) << 32) | (t.alloc_count as i64 & 0xFFFF_FFFF)
            }
            _ => 0,
        }
    }

    /// Retorna o handle da root arena.
    pub fn root_arena(&self) -> i64 {
        self.root_arena_handle
    }

    /// Registra a type table (substitui conteúdo).
    pub fn set_type_table(&mut self, types: Vec<TypeShape>) {
        self.type_table = types;
    }

    /// Obtém o TypeShape para um type_id.
    pub fn get_type_shape(&self, type_id: i64) -> Option<&TypeShape> {
        self.type_table.get(type_id as usize)
    }

    // ── Recursion depth ──────────────────────────────────

    /// Incrementa o contador de profundidade e retorna o novo valor.
    pub fn depth_inc(&self) -> u32 {
        let d = self.call_depth.get() + 1;
        self.call_depth.set(d);
        d
    }

    /// Decrementa o contador de profundidade.
    /// Saturating sub — não faz underflow abaixo de 0.
    pub fn depth_dec(&self) {
        let cur = self.call_depth.get();
        self.call_depth.set(cur.saturating_sub(1));
    }

    /// Retorna o contador de profundidade atual.
    pub fn depth_get(&self) -> u32 {
        self.call_depth.get()
    }

    /// Define o limite de profundidade de recursão.
    pub fn depth_set_limit(&self, limit: u32) {
        self.depth_limit.set(limit);
    }

    /// Retorna o limite de profundidade de recursão.
    pub fn depth_limit(&self) -> u32 {
        self.depth_limit.get()
    }

    /// Marca que houve overflow de profundidade.
    pub fn set_overflowed(&self) {
        self.overflowed.set(true);
    }

    /// Retorna se houve overflow de profundidade.
    pub fn overflowed(&self) -> bool {
        self.overflowed.get()
    }

    /// Reseta o estado de depth para uma nova execução.
    /// Zera `call_depth` e limpa a flag `overflowed`.
    pub fn reset_depth(&self) {
        self.call_depth.set(0);
        self.overflowed.set(false);
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper: converte `rt: i64` (FFI) para `&Runtime` (imutável) com null-check.
///
/// Para FFIs que só precisam de `&Runtime` (depth tracking usa `Cell`,
/// não precisa de `&mut`). Mesmo hardening que `deref_runtime`.
///
/// # Safety
/// `rt` deve ser um ponteiro válido obtido de `Box::into_raw` ou similar.
pub(crate) unsafe fn deref_runtime_ref(rt: i64) -> &'static Runtime {
    if rt == 0 {
        panic!(
            "kata_rt: deref de Runtime null (0) — ponteiro não-inicializado escapou do typeck; isto é um bug do compilador, não do seu código"
        );
    }
    unsafe { &*(rt as *const Runtime) }
}

/// Helper: converte `rt: i64` (FFI) para `&mut Runtime` com null-check.
///
/// Hardening do boundary FFI: `0` (null — Runtime não-inicializado que
/// escapou do typeck) é recusado com panic **unwind** e mensagem clara.
/// Antes: deref cego → SIGSEGV (non-unwinding), matando o processo
/// hospedeiro sem diagnóstico.
///
/// # Safety
/// `rt` deve ser um ponteiro válido obtido de `Box::into_raw` ou similar.
pub(crate) unsafe fn deref_runtime(rt: i64) -> &'static mut Runtime {
    if rt == 0 {
        panic!(
            "kata_rt: deref de Runtime null (0) — ponteiro não-inicializado escapou do typeck; isto é um bug do compilador, não do seu código"
        );
    }
    unsafe { &mut *(rt as *mut Runtime) }
}

/// Aloca um novo `Runtime` no heap e retorna o ponteiro como `i64`.
///
/// FFI C-ABI para o shim C do AOT — o JIT driver usa `Box::new(Runtime::new())`
/// diretamente, mas o shim C não tem acesso ao struct. Esta função expõe
/// a construção para código nativo.
///
/// O caller é responsável por liberar o Runtime (`Box::from_raw`) após
/// a execução. Para o shim C do AOT, o processo termina após a execução,
/// então o leak é aceitável (OS reclama a memória).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_runtime_new() -> i64 {
    let rt = Box::new(Runtime::new());
    Box::into_raw(rt) as i64
}

// ── FFI: Recursion depth ────────────────────────────────────────────

/// `kata_rt_depth_inc(rt: i64) -> i64` — incrementa profundidade, retorna novo valor.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_depth_inc(rt: i64) -> i64 {
    let rt = unsafe { deref_runtime_ref(rt) };
    rt.depth_inc() as i64
}

/// `kata_rt_depth_dec(rt: i64) -> ()` — decrementa profundidade.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_depth_dec(rt: i64) {
    let rt = unsafe { deref_runtime_ref(rt) };
    rt.depth_dec();
}

/// `kata_rt_depth_get(rt: i64) -> i64` — retorna profundidade atual.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_depth_get(rt: i64) -> i64 {
    let rt = unsafe { deref_runtime_ref(rt) };
    rt.depth_get() as i64
}

/// `kata_rt_depth_set_limit(rt: i64, limit: i64) -> ()` — define limite de profundidade.
/// Chamada pela FFI `set_recursion_limit` em `stdlib/config.kata`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_depth_set_limit(rt: i64, limit: i64) {
    let rt = unsafe { deref_runtime_ref(rt) };
    rt.depth_set_limit(limit as u32);
}

/// `kata_rt_set_overflowed(rt: i64) -> ()` — marca overflow de profundidade.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_set_overflowed(rt: i64) {
    let rt = unsafe { deref_runtime_ref(rt) };
    rt.set_overflowed();
}

/// `kata_rt_overflowed(rt: i64) -> i64` — retorna 1 se houve overflow, 0 caso contrário.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_overflowed(rt: i64) -> i64 {
    let rt = unsafe { deref_runtime_ref(rt) };
    if rt.overflowed() {
        1
    } else {
        0
    }
}

/// `kata_rt_depth_get_limit(rt: i64) -> i64` — retorna o limite de profundidade.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_depth_get_limit(rt: i64) -> i64 {
    let rt = unsafe { deref_runtime_ref(rt) };
    rt.depth_limit() as i64
}

/// `kata_rt_reset_depth(rt: i64) -> ()` — reseta depth e overflow para nova execução.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_reset_depth(rt: i64) {
    let rt = unsafe { deref_runtime_ref(rt) };
    rt.reset_depth();
}
