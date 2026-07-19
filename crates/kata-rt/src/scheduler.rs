//! Scheduler — coordena fibers e arenas em árvore hierárquica.
//!
//! Scheduler com yield (suspend/resume via wasmtime-fiber)
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
//!
//! **Estrutura do módulo:** o struct `Scheduler` + impl ficam aqui; a
//! camada FFI (TLS + funções `extern "C"`) está em [`ffi`]. Os TLS de
//! yield/pending spawns são `pub(super)` para o impl acessar.

pub(crate) mod ffi;

// Re-exports da camada FFI — `lib.rs` continua importando os mesmos símbolos.
pub use ffi::{
    DEADLOCK_SENTINEL, TIMEOUT_SENTINEL, kata_rt_run, kata_rt_scheduler_init,
    kata_rt_set_test_timeout, kata_rt_spawn, kata_rt_yield, kata_rt_yield_check, reset_scheduler,
};

use std::collections::{HashMap, VecDeque};

use crate::arena::{kata_rt_arena_create, kata_rt_arena_destroy};
use crate::channel::{can_recv, can_send};
use crate::fiber::{KataFiber, SpawnArgs, YieldReason};

use ffi::{HAS_READY_FIBER, PENDING_SPAWNS, YIELD_COUNTER, YIELD_INTERVAL};

/// Identificador de fiber no scheduler.
pub(crate) type FiberId = u64;

/// Razão pela qual um fiber está bloqueado.
#[derive(Debug, Clone)]
#[allow(dead_code)] // WaitingOnFiber é para fases futuras
#[allow(clippy::enum_variant_names)] // Prefixo WaitingOn é intencional
pub(crate) enum BlockReason {
    /// Esperando mensagem em canal (recv). `i64` = handle do canal.
    WaitingOnChannel(i64),
    /// Esperando espaço em canal (send). `i64` = handle do canal.
    WaitingOnChannelSend(i64),
    /// Esperando select. `Vec<i64>` = handles, `Option<Instant>` = deadline
    /// de timeout (None = sem timeout).
    WaitingOnSelect(Vec<i64>, Option<std::time::Instant>),
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
    /// `Suspend` ptr capturado após o `resume()` que suspendeu o fiber.
    /// Usado para re-setar `CURRENT_SUSPEND` antes do próximo `resume()` —
    /// o `SuspendGuard` do trampoline assume disciplina de pilha, mas
    /// fibers suspendem/retomam em ordem arbitrária. `None` se o fiber
    /// ainda não executou (primeiro resume) ou completou sem suspender.
    suspend_ptr: Option<crate::fiber::SuspendPtr>,
    /// Snapshot do `LOG_CONFIG` do pai no momento do spawn (herança β).
    /// Setado no `LOG_CONFIG` TLS antes de cada `resume()` deste fiber.
    log_config: Option<crate::log::LogConfig>,
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
        // Snapshot do LOG_CONFIG do pai (herança β). Copia a config atual
        // do TLS para o fiber filho. Mudanças no pai após o spawn não
        // propagam para filhos já spawnados.
        let log_config = crate::log::snapshot_log_config();
        let fiber = KataFiber::new(fiber_arena)?;
        let id = self.next_id;
        self.next_id += 1;
        self.fibers.insert(
            id,
            FiberEntry {
                fiber: std::mem::ManuallyDrop::new(fiber),
                spawn_args: SpawnArgs {
                    fn_ptr,
                    caller_arena,
                    args_ptr,
                    fiber_arena,
                },
                parent_id,
                children: Vec::new(),
                completed: false,
                suspend_ptr: None,
                log_config,
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
                    Err(YieldReason::WaitingOnSelect(handles, deadline)) => {
                        self.blocked
                            .insert(fiber_id, BlockReason::WaitingOnSelect(handles, deadline));
                    }
                    Err(YieldReason::Timeout) => {
                        // Timeout de teste — drenar fibers sem dropar (pitfall
                        // #43: wasmtime-fiber panica no Drop de fiber não-completado)
                        // e retornar Err("timeout"). `kata_rt_run` mapeia para
                        // `TIMEOUT_SENTINEL`. Mesmo padrão do deadlock (linha 249).
                        self.fibers.drain();
                        return Err("timeout".to_string());
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
                // Verificar se há fibers com deadline de timeout pendente.
                // Se sim, dormir até o deadline mais próximo e tentar novamente.
                // Se nenhum fiber tem deadline, é deadlock real.
                let earliest_deadline = self
                    .blocked
                    .values()
                    .filter_map(|reason| {
                        if let BlockReason::WaitingOnSelect(_, Some(dl)) = reason {
                            Some(*dl)
                        } else {
                            None
                        }
                    })
                    .min();

                if let Some(deadline) = earliest_deadline {
                    let now = std::time::Instant::now();
                    if deadline > now {
                        std::thread::sleep(deadline - now);
                    }
                    // Após dormir, fazer wake_pass para acordar fibers cujo
                    // deadline expirou ou cujo canal recebeu dado.
                    self.wake_pass();
                    continue;
                }

                // Sem deadlines — deadlock real.
                let n = self.blocked.len();
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
        let log_config = entry.log_config.clone();
        // Re-setar `CURRENT_SUSPEND` antes de cada resume(). O `SuspendGuard`
        // do trampoline assume disciplina de pilha (save/restore nested),
        // mas fibers suspendem/retomam em ordem arbitrária: quando A suspende
        // e B resumiu entre-temps, `CURRENT_SUSPEND` ainda contém o `Suspend`
        // de B. O trampoline publica o `Suspend` ptr em `LAST_SUSPEND_PTR`
        // (TLS) a cada execução; o scheduler captura após `resume()` retornar
        // `Err(YieldReason)` e re-seta `CURRENT_SUSPEND` aqui no próximo
        // resume. No primeiro resume (suspend_ptr=None), o trampoline seta
        // `CURRENT_SUSPEND` via `SuspendGuard`.
        if let Some(suspend) = entry.suspend_ptr {
            crate::fiber::set_current_suspend(suspend);
        }
        // Restaura LOG_CONFIG do snapshot do fiber (herança β).
        crate::log::restore_log_config(log_config);
        self.current_fiber = Some(fiber_id);
        let has_ready = !self.run_queue.is_empty();
        HAS_READY_FIBER.with(|h| h.set(has_ready));
        YIELD_COUNTER.with(|c| c.set(YIELD_INTERVAL));
        let result = entry.fiber.resume(spawn_args);
        self.current_fiber = None;
        // Se o fiber suspendeu, capturar o `Suspend` ptr publicado pelo
        // trampoline para re-setar `CURRENT_SUSPEND` no próximo resume.
        if result.is_err() {
            let suspend_ptr = crate::fiber::take_last_suspend();
            if let Some(entry) = self.fibers.get_mut(&fiber_id) {
                entry.suspend_ptr = suspend_ptr;
            }
        }
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
                BlockReason::WaitingOnSelect(handles, deadline) => {
                    // Select: algum canal tem dado OU deadline expirou.
                    if handles.iter().any(|h| can_recv(*h)) {
                        Some(id)
                    } else if let Some(dl) = deadline {
                        if std::time::Instant::now() >= *dl {
                            Some(id)
                        } else {
                            None
                        }
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
