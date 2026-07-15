//! Scheduler — coordena fibers e arenas em árvore hierárquica.
//!
//! Pré-11: o scheduler rastreia a **árvore** de fibers (parent_id/children),
//! não uma fila plana. A destruição é bottom-up: um fiber só é destruído
//! (e sua arena liberada) quando termina **e** todos os filhos terminaram.
//! A arena raiz é criada em `new()` e destruída quando o fiber raiz é
//! destruído (todos os fibers terminaram).
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
    /// Fiber pai na árvore (None = raiz / entry point).
    #[allow(dead_code)]
    parent_id: Option<FiberId>,
    /// Fibers filhos que ainda não terminaram.
    children: Vec<FiberId>,
    /// Fiber terminou execução (`resume()` retornou).
    completed: bool,
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
    /// Arena raiz — criada em `new()`, destruída quando o fiber raiz
    /// (sem pai) é destruído. Substitui a antiga arena global (handle 0).
    root_arena: i64,
}

impl Scheduler {
    /// Cria um scheduler vazio com a arena raiz alocada.
    pub(crate) fn new() -> Self {
        let root_arena = kata_rt_arena_create();
        Scheduler {
            run_queue: VecDeque::new(),
            blocked: std::collections::HashMap::new(),
            current_fiber: None,
            fibers: std::collections::HashMap::new(),
            next_id: 0,
            root_arena,
        }
    }

    /// Cria um fiber com arena própria e o enfileira para execução.
    ///
    /// O scheduler cria a arena do fiber (`kata_rt_arena_create`), passa o
    /// handle no `SpawnArgs.fiber_arena`. O `caller_arena` é a arena do pai
    /// (ou a arena raiz para o entry point).
    ///
    /// Registra `current_fiber` como pai do novo fiber na árvore.
    ///
    /// Retorna o `FiberId` do fiber criado.
    pub(crate) fn spawn(
        &mut self,
        fn_ptr: i64,
        caller_arena: i64,
        args_ptr: i64,
    ) -> Result<FiberId, String> {
        let fiber_arena = kata_rt_arena_create();
        let parent_id = self.current_fiber;
        let spawn_args = SpawnArgs {
            fn_ptr,
            caller_arena,
            args_ptr,
            fiber_arena,
        };
        let fiber = KataFiber::new(fiber_arena)?;
        let id = self.next_id;
        self.next_id += 1;
        self.fibers.insert(
            id,
            FiberEntry {
                fiber,
                spawn_args,
                parent_id,
                children: Vec::new(),
                completed: false,
            },
        );
        // Registrar este fiber como filho do pai.
        if let Some(pid) = parent_id
            && let Some(parent) = self.fibers.get_mut(&pid)
        {
            parent.children.push(id);
        }
        self.run_queue.push_back(id);
        Ok(id)
    }

    /// Executa o próximo fiber da run_queue.
    ///
    /// O fiber executa até completar (sem `yield` em Fase 10), retorna `i64`.
    /// Após completar, marca `completed = true` e tenta destruir bottom-up.
    ///
    /// Retorna o resultado do fiber, ou 0 se a run_queue está vazia.
    pub(crate) fn run(&mut self) -> i64 {
        while let Some(fiber_id) = self.run_queue.pop_front() {
            let Some(entry) = self.fibers.get_mut(&fiber_id) else {
                continue;
            };
            self.current_fiber = Some(fiber_id);
            let spawn_args = entry.spawn_args;
            let result = entry.fiber.resume(spawn_args).unwrap_or(0);
            self.current_fiber = None;

            // Marca como completado.
            let entry = self
                .fibers
                .get_mut(&fiber_id)
                .expect("fiber_id must exist in fibers map");
            entry.completed = true;

            // Tentar destruir este fiber e propagar bottom-up.
            self.try_destroy(fiber_id);

            return result;
        }
        0 // run_queue vazia
    }

    /// Tenta destruir um fiber completado. Se o pai também está completado
    /// e sem filhos, propaga a destruição recursivamente (bottom-up).
    ///
    /// Um fiber só é destruído quando `completed && children.is_empty()`.
    /// O fiber raiz (sem pai) destrói a `root_arena` ao ser destruído.
    fn try_destroy(&mut self, fiber_id: FiberId) {
        let should_destroy = {
            let Some(entry) = self.fibers.get(&fiber_id) else {
                return;
            };
            entry.completed && entry.children.is_empty()
        };
        if !should_destroy {
            return;
        }

        let entry = self
            .fibers
            .remove(&fiber_id)
            .expect("fiber_id must exist if should_destroy was true");
        kata_rt_arena_destroy(entry.fiber.arena_handle);

        // Remover este fiber da lista de children do pai e propagar.
        let parent_id = entry.parent_id;
        if let Some(pid) = parent_id {
            if let Some(parent) = self.fibers.get_mut(&pid) {
                parent.children.retain(|&c| c != fiber_id);
                // Propagar: se o pai também está completado e sem filhos, destruir.
                self.try_destroy(pid);
            }
        } else {
            // Sem pai = fiber raiz. Todos os fibers terminaram.
            // Destruir arena raiz.
            kata_rt_arena_destroy(self.root_arena);
        }
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
#[allow(dead_code)]
pub(crate) fn reset_scheduler() {
    SCHEDULER.with(|s| {
        s.borrow_mut().take();
    });
}

/// Inicializa o scheduler thread-local e cria a arena raiz.
///
/// Retorna o handle da arena raiz para o codegen usar como `caller_arena`
/// no entry point (Pré-11). Antes retornava 1 (sucesso).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_scheduler_init() -> i64 {
    SCHEDULER.with(|s| {
        let scheduler = Scheduler::new();
        let root_arena = scheduler.root_arena;
        *s.borrow_mut() = Some(scheduler);
        root_arena
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
