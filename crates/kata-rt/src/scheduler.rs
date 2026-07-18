//! Scheduler — coordena fibers e arenas em árvore hierárquica.
//!
//! Fase 4 do Fio 11: scheduler com yield (suspend/resume via wasmtime-fiber)
//! e structured concurrency (Action espera forks completarem).
//!
//! O scheduler permanece single-threaded (Decisão A). `thread_local!` como
//! antes. O run loop agora processa múltiplos fibers em round-robin, com
//! yield cooperativo e blocking em canais.
//!
//! **Conflito de borrow:** durante `resume()`, a função JIT pode chamar FFIs
//! (`kata_rt_yield`, `kata_rt_channel_recv/send`). Estas FFIs **não acessam
//! o scheduler** — apenas acessam o `Suspend` via TLS (ver `fiber.rs`) e
//! chamam `suspend(YieldReason)`. O scheduler interpreta o `YieldReason`
//! quando `resume()` retorna `Err(YieldReason)`.
//!
//! **Wake mechanism:** após cada `resume()`, o scheduler faz um "wake pass":
//! percorre `blocked` e chama `can_recv`/`can_send` (verificação sem consumo)
//! para ver se a operação pode prosseguir. Se sim, move o fiber de
//! `blocked` para `run_queue`.

use std::collections::{HashMap, VecDeque};

use crate::arena::{kata_rt_arena_create, kata_rt_arena_destroy};
use crate::channel::{can_recv, can_send};
use crate::fiber::{KataFiber, SpawnArgs, YieldReason};

/// Identificador de fiber no scheduler.
pub(crate) type FiberId = u64;

/// Razão pela qual um fiber está bloqueado.
#[derive(Debug, Clone)]
#[allow(dead_code)] // WaitingOnFiber e WaitingOnSelect são para fases futuras
#[allow(clippy::enum_variant_names)] // Prefixo WaitingOn é intencional
pub(crate) enum BlockReason {
    /// Esperando mensagem em canal (recv). `i64` = handle do canal.
    WaitingOnChannel(i64),
    /// Esperando espaço em canal (send). `i64` = handle do canal.
    WaitingOnChannelSend(i64),
    /// Esperando select (Fase 6). `Vec<i64>` = handles.
    WaitingOnSelect(Vec<i64>),
    /// Esperando outro fiber terminar.
    WaitingOnFiber(FiberId),
}

/// Entry do fiber no scheduler.
struct FiberEntry {
    /// `ManuallyDrop` porque wasmtime-fiber panica se dropado sem
    /// completar. Em deadlock, o scheduler esquece os fibers via
    /// `ManuallyDrop::drop` seletivo.
    fiber: std::mem::ManuallyDrop<KataFiber>,
    /// SpawnArgs guardados para passar no `resume()`.
    spawn_args: SpawnArgs,
    /// Fiber pai na árvore (None = raiz / entry point).
    parent_id: Option<FiberId>,
    /// Fibers filhos que ainda não terminaram.
    children: Vec<FiberId>,
    /// Fiber terminou execução (`resume()` retornou Ok).
    completed: bool,
}

/// Scheduler de fibers — coordena execução, arenas e yield.
pub(crate) struct Scheduler {
    run_queue: VecDeque<FiberId>,
    /// Fibers bloqueados esperando canal/select.
    blocked: HashMap<FiberId, BlockReason>,
    current_fiber: Option<FiberId>,
    fibers: HashMap<FiberId, FiberEntry>,
    next_id: u64,
    /// Arena raiz — criada em `new()`, destruída quando o fiber raiz
    /// (sem pai) é destruído.
    root_arena: i64,
    /// Resultado do fiber raiz — guardado quando o fiber raiz completa.
    root_result: i64,
}

impl Scheduler {
    /// Cria um scheduler vazio com a arena raiz alocada.
    pub(crate) fn new() -> Self {
        let root_arena = kata_rt_arena_create();
        Scheduler {
            run_queue: VecDeque::new(),
            blocked: HashMap::new(),
            current_fiber: None,
            fibers: HashMap::new(),
            next_id: 0,
            root_arena,
            root_result: 0,
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
                fiber: std::mem::ManuallyDrop::new(fiber),
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

    /// Run loop principal — executa fibers até todos completarem.
    ///
    /// 1. Pop da run_queue, resume, interpretar YieldReason
    /// 2. run_queue vazia — wake pass (verificar blocked)
    /// 3. run_queue vazia + blocked não vazio = deadlock (Err)
    /// 4. run_queue vazia + blocked vazio = todos terminaram (Ok)
    pub(crate) fn run(&mut self) -> Result<i64, String> {
        loop {
            // 1. Tentar executar próximo fiber pronto.
            if let Some(fiber_id) = self.run_queue.pop_front() {
                let result = self.resume_fiber(fiber_id);
                match result {
                    Ok(ret) => {
                        // Fiber completou.
                        if let Some(entry) = self.fibers.get_mut(&fiber_id) {
                            entry.completed = true;
                        }
                        // Se é o fiber raiz, guardar resultado.
                        let is_root = self
                            .fibers
                            .get(&fiber_id)
                            .map(|e| e.parent_id.is_none())
                            .unwrap_or(false);
                        if is_root {
                            self.root_result = ret;
                        }
                        // try_destroy (só destrói se completed && children.is_empty)
                        self.try_destroy(fiber_id);
                    }
                    Err(YieldReason::Cooperative) => {
                        // Yield point — volta para run_queue.
                        self.run_queue.push_back(fiber_id);
                    }
                    Err(YieldReason::WaitingOnChannel(handle)) => {
                        // Bloqueia em recv.
                        self.blocked
                            .insert(fiber_id, BlockReason::WaitingOnChannel(handle));
                    }
                    Err(YieldReason::WaitingOnChannelSend(handle)) => {
                        // Bloqueia em send.
                        self.blocked
                            .insert(fiber_id, BlockReason::WaitingOnChannelSend(handle));
                    }
                    Err(YieldReason::WaitingOnSelect(handles)) => {
                        // Bloqueia em select (Fase 6 — mas YieldReason já existe).
                        self.blocked
                            .insert(fiber_id, BlockReason::WaitingOnSelect(handles));
                    }
                    Err(YieldReason::Done) => {
                        unreachable!("YieldReason::Done não deve ser retornado por resume()")
                    }
                }
                // Após cada resume, fazer wake pass para acordar fibers
                // blocked que agora podem prosseguir.
                self.wake_pass();
                // Drenar spawns pendentes (enfileirados por kata_rt_spawn
                // durante resume). O fiber_id que acabou de executar é
                // o pai dos spawns pendentes.
                self.drain_pending_spawns(fiber_id);
                continue;
            }

            // 2. run_queue vazia — verificar blocked.
            if !self.blocked.is_empty() {
                // Sem timers na Fase 4. Deadlock detection.
                let n = self.blocked.len();
                // Fibers suspensos não completarão. Seus `KataFiber` estão
                // em `ManuallyDrop` — o `drain()` dropa `FiberEntry` mas
                // NÃO dropa o `KataFiber` interno, evitando o panic
                // "fiber dropped without finishing" do wasmtime-fiber.
                // As arenas dos fibers são vazadas (não destruídas), mas
                // o scheduler será resetado/dropado em seguida.
                self.fibers.drain();
                return Err(format!("deadlock: {n} fibers bloqueados sem progresso"));
            }

            // 3. run_queue vazia, blocked vazia — todos terminaram.
            // Destruir arena raiz.
            kata_rt_arena_destroy(self.root_arena);
            return Ok(self.root_result);
        }
    }

    /// Resume um fiber e retorna o resultado.
    ///
    /// Retorna `Ok(i64)` se completou, `Err(YieldReason)` se suspendeu.
    fn resume_fiber(&mut self, fiber_id: FiberId) -> Result<i64, YieldReason> {
        let Some(entry) = self.fibers.get(&fiber_id) else {
            return Ok(0); // fiber já destruído
        };
        let spawn_args = entry.spawn_args;
        self.current_fiber = Some(fiber_id);
        let result = entry.fiber.resume(spawn_args);
        self.current_fiber = None;
        result
    }

    /// Wake pass — percorre `blocked` e verifica se a operação bloqueada
    /// agora pode prosseguir (can_recv/can_send). Move fibers acordados
    /// de `blocked` para `run_queue`.
    fn wake_pass(&mut self) {
        if self.blocked.is_empty() {
            return;
        }
        // Coletar fibers para acordar.
        let to_wake: Vec<FiberId> = self
            .blocked
            .iter()
            .filter_map(|(&id, reason)| match reason {
                BlockReason::WaitingOnChannel(handle) => {
                    if can_recv(*handle) {
                        Some(id)
                    } else {
                        None
                    }
                }
                BlockReason::WaitingOnChannelSend(handle) => {
                    if can_send(*handle) {
                        Some(id)
                    } else {
                        None
                    }
                }
                BlockReason::WaitingOnSelect(handles) => {
                    // Fase 6 — select. Verificar se algum canal tem dado.
                    if handles.iter().any(|h| can_recv(*h)) {
                        Some(id)
                    } else {
                        None
                    }
                }
                BlockReason::WaitingOnFiber(_) => None,
            })
            .collect();
        for id in to_wake {
            self.blocked.remove(&id);
            self.run_queue.push_back(id);
        }
    }

    /// Drena a lista de spawns pendentes (TLS `PENDING_SPAWNS`).
    ///
    /// `kata_rt_spawn` chamado de dentro de um fiber enfileira em
    /// `PENDING_SPAWNS` para evitar double-borrow do SCHEDULER. Este
    /// método é chamado após cada `resume()` para criar os fibers
    /// enfileirados. O `parent_id` é o fiber que acabou de executar
    /// (que chamou `kata_rt_spawn`).
    fn drain_pending_spawns(&mut self, parent_id: FiberId) {
        let pending: Vec<(i64, i64, i64)> =
            PENDING_SPAWNS.with(|p| p.borrow_mut().drain(..).collect());
        for (fn_ptr, caller_arena, args_ptr) in pending {
            // Usar parent_id como pai, não current_fiber (que é None
            // neste ponto).
            self.current_fiber = Some(parent_id);
            if let Err(e) = self.spawn(fn_ptr, caller_arena, args_ptr) {
                eprintln!("kata_rt_spawn: erro ao drenar spawn: {e}");
            }
            self.current_fiber = None;
        }
    }

    /// Tenta destruir um fiber completado. Se o pai também está completado
    /// e sem filhos, propaga a destruição recursivamente (bottom-up).
    ///
    /// Um fiber só é destruído quando `completed && children.is_empty()`.
    /// O fiber raiz (sem pai) destrói a `root_arena` ao ser destruído — mas
    /// só quando **todos** os fibers terminaram (`self.fibers.is_empty()`),
    /// pois a `root_arena` é compartilhada entre todos os fibers raiz.
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

        let mut entry = self
            .fibers
            .remove(&fiber_id)
            .expect("fiber_id must exist if should_destroy was true");
        let arena_handle = entry.fiber.arena_handle;
        let parent_id = entry.parent_id;
        kata_rt_arena_destroy(arena_handle);
        // Drop manual do fiber — `ManuallyDrop` não dropa automaticamente.
        // O fiber completou normalmente, então o Drop do wasmtime-fiber
        // não vai panicar.
        // SAFETY: fiber completou (completed=true), não está suspenso.
        unsafe {
            std::mem::ManuallyDrop::drop(&mut entry.fiber);
        }

        // Remover este fiber da lista de children do pai e propagar.
        if let Some(pid) = parent_id
            && let Some(parent) = self.fibers.get_mut(&pid)
        {
            parent.children.retain(|&c| c != fiber_id);
            // Propagar: se o pai também está completado e sem filhos, destruir.
            self.try_destroy(pid);
        }
        // Não destruir root_arena aqui — só quando todos os fibers terminaram.
        // O run loop chama `kata_rt_arena_destroy(self.root_arena)` no fim.
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ── Funções C-ABI para o codegen ─────────────────────────────────────

// Scheduler thread-local — 1 por thread (single-threaded).
thread_local! {
    static SCHEDULER: std::cell::RefCell<Option<Scheduler>> = const { std::cell::RefCell::new(None) };
}

// Spawns pendentes — enfileirados por `kata_rt_spawn` quando chamado
// de dentro de um fiber (durante `resume()`). O scheduler drena esta
// lista após cada `resume()` no `run()`.
//
// Isto evita o double-borrow do SCHEDULER: durante `resume()`, o
// scheduler já tem o `RefCell` emprestado (`borrow_mut` em `run()`).
// Se a função JIT chamar `kata_rt_spawn`, um segundo `borrow_mut`
// causaria panic. Em vez disso, `kata_rt_spawn` detecta que está
// dentro de um fiber (Suspend em TLS) e enfileira o spawn aqui.
thread_local! {
    static PENDING_SPAWNS: std::cell::RefCell<Vec<(i64, i64, i64)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Reseta o scheduler thread-local. Chamado entre execuções de teste.
pub fn reset_scheduler() {
    SCHEDULER.with(|s| {
        s.borrow_mut().take();
    });
    PENDING_SPAWNS.with(|p| {
        p.borrow_mut().clear();
    });
}

/// Inicializa o scheduler thread-local e cria a arena raiz.
///
/// Retorna o handle da arena raiz para o codegen usar como `caller_arena`
/// no entry point.
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
/// Se chamado de dentro de um fiber (durante `resume()`), enfileira o
/// spawn em `PENDING_SPAWNS` em vez de acessar o SCHEDULER diretamente —
/// evita double-borrow do `RefCell`. O scheduler drena a lista após
/// cada `resume()`.
///
/// Retorna o `FiberId` (i64). Quando enfileirado, retorna 0 (o FiberId
/// será atribuído quando o scheduler drenar).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_spawn(fn_ptr: i64, caller_arena: i64, args_ptr: i64) -> i64 {
    if crate::fiber::is_in_fiber() {
        // Dentro de fiber — enfileirar em PENDING_SPAWNS.
        PENDING_SPAWNS.with(|p| {
            p.borrow_mut().push((fn_ptr, caller_arena, args_ptr));
        });
        0
    } else {
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
}

/// Executa o scheduler até todos os fibers completarem.
///
/// Retorna o resultado do fiber raiz (i64). Se deadlock for detectado,
/// imprime a mensagem no stderr e retorna `DEADLOCK_SENTINEL`.
///
/// Não faz `panic!` porque `extern "C"` é `nounwind` — um panic aqui
/// aborta com SIGABRT (non-unwinding). O chamador (driver/teste)
/// verifica o valor de retorno.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_run() -> i64 {
    let result = SCHEDULER.with(|s| {
        let mut s = s.borrow_mut();
        let scheduler = s.as_mut().expect("scheduler não inicializado");
        scheduler.run()
    });
    match result {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("kata_rt_run: {msg}");
            DEADLOCK_SENTINEL
        }
    }
}

/// Valor retornado por `kata_rt_run` quando deadlock é detectado.
/// Permite que o chamador distinga deadlock de um resultado legítimo.
pub const DEADLOCK_SENTINEL: i64 = i64::MIN + 1;

/// Suspende o fiber atual com `YieldReason::Cooperative`.
///
/// Chamada pelo codegen em yield points (Fase 7) ou por código Kata que
/// explicitamente cede CPU. Não acessa o scheduler — apenas suspende via
/// TLS `Suspend`. O scheduler interpreta o `YieldReason` quando `resume()`
/// retorna `Err(YieldReason::Cooperative)` e coloca o fiber de volta na
/// run_queue.
///
/// Se chamada fora de um fiber (sem `Suspend` em TLS), é no-op.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_yield() {
    crate::fiber::with_suspend(|suspend| {
        suspend.suspend(YieldReason::Cooperative);
    });
}
