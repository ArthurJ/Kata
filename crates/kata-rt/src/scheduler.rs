//! Scheduler — coordena fibers e arenas.
//!
//! Em Fase 10, o scheduler é simples: `spawn` cria um fiber com sua arena,
//! `run` executa o fiber e destrói a arena. `run_queue` sempre tem 1 fiber
//! (sem concorrência em Fase 10 — `yield` e `spawn` enfileirador vêm no Fio 11).
//!
//! O scheduler é opaco ao tipo de retorno — só repassa `i64`. O caller
//! (codegen) sabe o `ret_ty` em compile time e faz bitcast se Float.

use std::collections::VecDeque;

use crate::arena::{kata_rt_arena_create, kata_rt_arena_destroy};
use crate::fiber::{KataFiber, SpawnArgs};

/// Identificador de fiber no scheduler.
pub(crate) type FiberId = u64;

/// Razão pela qual um fiber está bloqueado (não usado em Fase 10).
#[allow(dead_code)]
pub(crate) enum BlockReason {
    /// Esperando mensagem em canal (Fio 11+).
    WaitingOnChannel,
    /// Esperando outro fiber terminar.
    WaitingOnFiber(FiberId),
}

/// Entry do fiber no scheduler.
struct FiberEntry {
    fiber: KataFiber,
    /// SpawnArgs guardados para passar no `resume()`.
    spawn_args: SpawnArgs,
}

/// Scheduler de fibers — coordena execução, arenas e (futuramente) yield.
pub(crate) struct Scheduler {
    run_queue: VecDeque<FiberId>,
    /// Fibers bloqueados (vazio em Fase 10).
    #[allow(dead_code)]
    blocked: std::collections::HashMap<FiberId, BlockReason>,
    current_fiber: Option<FiberId>,
    fibers: std::collections::HashMap<FiberId, FiberEntry>,
    next_id: u64,
}

impl Scheduler {
    /// Cria um scheduler vazio.
    pub(crate) fn new() -> Self {
        Scheduler {
            run_queue: VecDeque::new(),
            blocked: std::collections::HashMap::new(),
            current_fiber: None,
            fibers: std::collections::HashMap::new(),
            next_id: 0,
        }
    }

    /// Cria um fiber com arena própria e o enfileira para execução.
    ///
    /// O scheduler cria a arena do fiber (`kata_rt_arena_create`), passa o
    /// handle no `SpawnArgs.fiber_arena`, e destrói a arena após `run`.
    ///
    /// Retorna o `FiberId` do fiber criado.
    pub(crate) fn spawn(
        &mut self,
        fn_ptr: i64,
        caller_arena: i64,
        args_ptr: i64,
    ) -> Result<FiberId, String> {
        let fiber_arena = kata_rt_arena_create();
        let spawn_args = SpawnArgs {
            fn_ptr,
            caller_arena,
            args_ptr,
            fiber_arena,
        };
        let fiber = KataFiber::new(fiber_arena)?;
        let id = self.next_id;
        self.next_id += 1;
        self.fibers.insert(id, FiberEntry { fiber, spawn_args });
        self.run_queue.push_back(id);
        Ok(id)
    }

    /// Executa o próximo fiber da run_queue.
    ///
    /// Em Fase 10, a run_queue sempre tem no máximo 1 fiber. O fiber executa
    /// até completar (sem `yield`), retorna `i64`, e o scheduler destrói a arena.
    ///
    /// Retorna o resultado do fiber, ou 0 se a run_queue está vazia.
    pub(crate) fn run(&mut self) -> i64 {
        while let Some(fiber_id) = self.run_queue.pop_front() {
            let Some(entry) = self.fibers.get_mut(&fiber_id) else {
                continue;
            };
            self.current_fiber = Some(fiber_id);
            let spawn_args = entry.spawn_args;
            let result = entry.fiber.resume(spawn_args).unwrap_or(0); // yield não usado em Fase 10
            self.current_fiber = None;

            // Destrói a arena do fiber e remove o fiber do scheduler.
            let entry = self
                .fibers
                .remove(&fiber_id)
                .expect("fiber_id must exist in fibers map");
            kata_rt_arena_destroy(entry.fiber.arena_handle);

            // Em Fase 10: run_queue sempre vazia após 1 fiber.
            // Em Fio 11: spawn pode adicionar à run_queue durante execução.
            return result;
        }
        0 // run_queue vazia
    }

    /// Suspende o fiber atual. Não chamado em Fase 10 (sem canais).
    #[allow(dead_code)]
    pub(crate) fn yield_(&mut self) {
        // Implementação real em Fio 11 — precisa de Suspend::suspend.
        // Em Fase 10, yield_ existe para satisfazer DoD 30 mas não é chamado.
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ── Funções C-ABI para o codegen ─────────────────────────────────────

// Scheduler thread-local — 1 por thread (Fase 10: single-threaded).
thread_local! {
    static SCHEDULER: std::cell::RefCell<Option<Scheduler>> = const { std::cell::RefCell::new(None) };
}

/// Reseta o scheduler thread-local. Chamado entre execuções de teste.
pub fn reset_scheduler() {
    SCHEDULER.with(|s| {
        s.borrow_mut().take();
    });
}

/// Inicializa o scheduler thread-local. Retorna 1 (sucesso) ou 0 (erro).
///
/// Chamado no prólogo de `__kata_entry`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_scheduler_init() -> i64 {
    SCHEDULER.with(|s| {
        *s.borrow_mut() = Some(Scheduler::new());
        1 // sucesso
    })
}

/// Cria um fiber com arena própria e o enfileira.
///
/// Chamado no `__kata_entry` quando encontra ActionCall definida pelo usuário.
///
/// Retorna o `FiberId` (i64).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_spawn(fn_ptr: i64, caller_arena: i64, args_ptr: i64) -> i64 {
    SCHEDULER.with(|s| {
        let mut s = s.borrow_mut();
        let scheduler = s.as_mut().expect("scheduler não inicializado");
        match scheduler.spawn(fn_ptr, caller_arena, args_ptr) {
            Ok(id) => id as i64,
            Err(_e) => {
                eprintln!("kata_rt_spawn: erro ao criar fiber: {_e}");
                0
            }
        }
    })
}

/// Executa o próximo fiber da run_queue.
///
/// Retorna o resultado do fiber (i64), ou 0 se run_queue vazia.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_run() -> i64 {
    SCHEDULER.with(|s| {
        let mut s = s.borrow_mut();
        let scheduler = s.as_mut().expect("scheduler não inicializado");
        scheduler.run()
    })
}

/// Suspende o fiber atual. Não chamado em Fase 10 (sem canais).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_yield() {
    SCHEDULER.with(|s| {
        let mut s = s.borrow_mut();
        let scheduler = s.as_mut().expect("scheduler não inicializado");
        scheduler.yield_();
    })
}
