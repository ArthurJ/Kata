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
    kata_rt_set_test_timeout, kata_rt_sleep, kata_rt_spawn, kata_rt_yield, kata_rt_yield_check,
    reset_scheduler,
};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

use crate::arena::{Arena, ArenaKind};
use crate::channel::{block_ipc_until_readable, can_recv, can_send, ipc_read_fd, is_ipc_handle};
use crate::fiber::{KataFiber, SpawnArgs, YieldReason};
use crate::file::{FILE_WOULD_BLOCK, collect_file_fds, try_select_files};
use crate::platform::{POLLIN, PollFd, poll_fds};
use crate::socket::{SOCKET_WOULD_BLOCK, collect_socket_fds, try_select_sockets};

// ── TLS para registry por-fiber (Fase 9) ───────────────────────────
// CURRENT_FIBER_ARENA: setado antes de resume(), lido por kata_rt_file_open
//   para determinar se o arquivo é fiber-local (arena do fiber) ou global.
// FIBER_OPEN_FILES: Vec<i64> swap in/out de FiberEntry.open_files antes/depois
//   de resume(). kata_rt_file_open registra aqui se fiber-local;
//   try_destroy fecha os handles pendentes.

thread_local! {
    pub(crate) static CURRENT_FIBER_ARENA: Cell<Option<i64>> = const { Cell::new(None) };
    pub(crate) static FIBER_OPEN_FILES: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
}

/// Identificador de fiber no scheduler.
pub(crate) type FiberId = u64;

/// Razão pela qual um fiber está bloqueado.
#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)] // Prefixo WaitingOn é intencional
pub(crate) enum BlockReason {
    /// Esperando mensagem em canal (recv). `i64` = handle do canal.
    WaitingOnChannel(i64),
    /// Esperando espaço em canal (send). `i64` = handle do canal.
    WaitingOnChannelSend(i64),
    /// Esperando select. Handles de canal e file separados.
    /// `deadline` de timeout (None = sem timeout).
    WaitingOnSelect {
        channel_handles: Vec<i64>,
        file_handles: Vec<i64>,
        socket_handles: Vec<i64>,
        deadline: Option<std::time::Instant>,
    },
    /// Esperando sleep cooperativo expirar. `Instant` = deadline.
    WaitingOnSleep(std::time::Instant),
    /// Esperando outro fiber terminar.
    #[allow(dead_code)] // variant usado em match arms mas não construído
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
    /// Handles de arquivos abertos pelo fiber (não-stdio, alocados na
    /// fiber_arena). Fechados em `try_destroy` se o fiber terminar sem
    /// chamar `close!()`. Swap in/out via `FIBER_OPEN_FILES` TLS.
    open_files: Vec<i64>,
}

/// Scheduler de fibers — coordena execução, arenas e yield.
pub(crate) struct Scheduler {
    run_queue: VecDeque<FiberId>,
    /// Fibers bloqueados esperando canal/select.
    blocked: HashMap<FiberId, BlockReason>,
    current_fiber: Option<FiberId>,
    fibers: HashMap<FiberId, FiberEntry>,
    next_id: u64,
    /// Arena raiz — criada pelo `Runtime::new()`, passada como parâmetro.
    #[allow(dead_code)] // campo reservado para futura utilização via Scheduler
    root_arena: i64,
    /// Resultado do fiber raiz — guardado quando o fiber raiz completa.
    root_result: i64,
    /// Yield cooperativo — contador decrementado no hot path de loops.
    /// Antes em `YIELD_COUNTER` TLS.
    pub(crate) yield_counter: i64,
    /// Snapshot booleano: há outra fiber pronta na run_queue.
    /// Antes em `HAS_READY_FIBER` TLS.
    pub(crate) has_ready_fiber: bool,
    /// Spawns pendentes — enfileirados por `kata_rt_spawn` quando chamado
    /// de dentro de um fiber. Antes em `PENDING_SPAWNS` TLS.
    /// Com o Runtime explícito, `kata_rt_spawn` pode acessar o scheduler
    /// diretamente via ponteiro, mas mantemos o campo para o caso de
    /// o fiber estar em resume() (borrow do scheduler via run()).
    pub(crate) pending_spawns: Vec<(i64, i64, i64, i64)>,
}

/// Intervalo de yield cooperativo — a cada N iterações, checar se há
/// outra fiber pronta.
pub(crate) const YIELD_INTERVAL: i64 = 1000;

impl Scheduler {
    /// Cria um scheduler vazio. A root arena é criada pelo `Runtime::new()`
    /// e passada como `root_arena_handle`.
    pub(crate) fn new(root_arena_handle: i64) -> Self {
        Scheduler {
            run_queue: VecDeque::new(),
            blocked: HashMap::new(),
            current_fiber: None,
            fibers: HashMap::new(),
            next_id: 0,
            root_arena: root_arena_handle,
            root_result: 0,
            yield_counter: YIELD_INTERVAL,
            has_ready_fiber: false,
            pending_spawns: Vec::new(),
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
        arenas: &mut Vec<ArenaKind>,
        fn_ptr: i64,
        rt: i64,
        caller_arena: i64,
        args_ptr: i64,
    ) -> Result<FiberId, String> {
        let fiber_arena = {
            let id = arenas.len() as i64;
            arenas.push(ArenaKind::Bump(Arena::new()));
            id
        };
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
                    rt,
                    caller_arena,
                    args_ptr,
                    fiber_arena,
                },
                parent_id,
                children: Vec::new(),
                completed: false,
                suspend_ptr: None,
                log_config,
                open_files: Vec::new(),
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
    pub(crate) fn run(&mut self, arenas: &mut Vec<ArenaKind>) -> Result<i64, String> {
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
                        self.try_destroy(arenas, fiber_id);
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
                    Err(YieldReason::WaitingOnSelect {
                        channel_handles,
                        file_handles,
                        socket_handles,
                        deadline,
                    }) => {
                        self.blocked.insert(
                            fiber_id,
                            BlockReason::WaitingOnSelect {
                                channel_handles,
                                file_handles,
                                socket_handles,
                                deadline,
                            },
                        );
                    }
                    Err(YieldReason::Sleep(deadline)) => {
                        self.blocked
                            .insert(fiber_id, BlockReason::WaitingOnSleep(deadline));
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
                self.drain_pending_spawns(arenas, fiber_id);
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
                    .filter_map(|reason| match reason {
                        BlockReason::WaitingOnSelect {
                            deadline: Some(dl), ..
                        } => Some(*dl),
                        BlockReason::WaitingOnSleep(dl) => Some(*dl),
                        _ => None,
                    })
                    .min();

                // Coletar handles IPC de fibers blocked (para poll com timeout).
                let ipc_handle = self.blocked.values().find_map(|reason| match reason {
                    BlockReason::WaitingOnChannel(handle) if is_ipc_handle(*handle) => {
                        Some(*handle)
                    }
                    BlockReason::WaitingOnSelect {
                        channel_handles, ..
                    } => channel_handles.iter().find(|h| is_ipc_handle(**h)).copied(),
                    _ => None,
                });

                // Coletar FDs de file e socket handles de fibers blocked em WaitingOnSelect.
                let mut file_fds: Vec<PollFd> = Vec::new();
                for reason in self.blocked.values() {
                    if let BlockReason::WaitingOnSelect {
                        file_handles,
                        socket_handles,
                        ..
                    } = reason
                    {
                        file_fds.extend(collect_file_fds(file_handles));
                        file_fds.extend(collect_socket_fds(socket_handles));
                    }
                }

                if let Some(deadline) = earliest_deadline {
                    let now = std::time::Instant::now();
                    let remaining = if deadline > now {
                        Some(deadline - now)
                    } else {
                        None
                    };

                    if let Some(remaining) = remaining {
                        let timeout_ms = remaining.as_millis() as i32;

                        // Poll unificado: IPC + file FDs.
                        if !file_fds.is_empty() {
                            // Adiciona o FD IPC ao poll set, se houver.
                            if let Some(handle) = ipc_handle {
                                // SAFETY: handle veio de um fiber blocked em canal IPC.
                                let fd = unsafe { ipc_read_fd(handle) };
                                if fd >= 0 {
                                    file_fds.push(PollFd {
                                        fd,
                                        events: POLLIN,
                                        revents: 0,
                                    });
                                }
                            }
                            // SAFETY: poll com timeout específico. Acorda quando
                            // qualquer FD (IPC ou file) tem dados ou timeout expira.
                            poll_fds(&mut file_fds, timeout_ms);
                        } else if let Some(handle) = ipc_handle {
                            // Sem file FDs, mas há IPC — poll IPC com timeout.
                            unsafe {
                                super::channel::ipc::poll_ipc_with_timeout(handle, timeout_ms);
                            }
                        } else {
                            std::thread::sleep(remaining);
                        }
                    }
                    // Após dormir (ou poll), fazer wake_pass para acordar fibers cujo
                    // deadline expirou ou cujo canal/arquivo recebeu dado.
                    self.wake_pass();
                    continue;
                }

                // Sem deadlines — verificar se algum fiber está bloqueado
                // em canal IPC ou file handle. Se sim, bloquear até ter dados.
                if let Some(handle) = ipc_handle {
                    // Bloqueia até o child OS process escrever no pipe.
                    // SAFETY: handle veio de um fiber blocked em canal IPC.
                    unsafe {
                        block_ipc_until_readable(handle);
                    }
                    // Após dados chegarem, fazer wake_pass e continuar.
                    self.wake_pass();
                    continue;
                }

                // Sem IPC, mas há file FDs bloqueados — poll blocking.
                if !file_fds.is_empty() {
                    // SAFETY: poll com timeout -1 (infinite). Acorda quando
                    // qualquer file FD tem dados.
                    poll_fds(&mut file_fds, -1);
                    self.wake_pass();
                    continue;
                }

                // Sem deadlines, sem IPC, sem file FDs — deadlock real.
                let n = self.blocked.len();
                self.fibers.drain();
                return Err(format!("deadlock: {n} fibers bloqueados sem progresso"));
            }

            // 3. run_queue vazia, blocked vazia — todos terminaram.
            // NÃO destruir a root arena aqui — o resultado (root_result)
            // pode ser um ponteiro para dados na root arena. A root arena
            // é destruída em `reset_scheduler` ou no `Drop` do Scheduler.
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

        // Fase 9: setar CURRENT_FIBER_ARENA e swap FIBER_OPEN_FILES.
        // kata_rt_file_open lê CURRENT_FIBER_ARENA para decidir se registra
        // o handle em FIBER_OPEN_FILES (fiber-local) ou OPEN_FILES (global).
        // FIBER_OPEN_FILES é swap in/out para evitar &mut Scheduler durante resume().
        let fiber_arena = spawn_args.fiber_arena;
        CURRENT_FIBER_ARENA.with(|c| c.set(Some(fiber_arena)));
        let saved_open_files = FIBER_OPEN_FILES.with(|r| {
            let mut borrow = r.borrow_mut();
            std::mem::take(&mut *borrow)
        });
        // Restore the fiber's open_files into TLS (may be non-empty if resuming
        // a suspended fiber that opened files before suspending).
        FIBER_OPEN_FILES.with(|r| {
            *r.borrow_mut() = saved_open_files;
        });

        let has_ready = !self.run_queue.is_empty();
        self.has_ready_fiber = has_ready;
        self.yield_counter = YIELD_INTERVAL;
        let result = entry.fiber.resume(spawn_args);
        self.current_fiber = None;

        // Fase 9: limpar CURRENT_FIBER_ARENA e recuperar FIBER_OPEN_FILES.
        CURRENT_FIBER_ARENA.with(|c| c.set(None));
        let recovered_open_files = FIBER_OPEN_FILES.with(|r| {
            let mut borrow = r.borrow_mut();
            std::mem::take(&mut *borrow)
        });
        // Devolve os open_files ao FiberEntry.
        if let Some(entry) = self.fibers.get_mut(&fiber_id) {
            entry.open_files = recovered_open_files;
        }

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
                BlockReason::WaitingOnSelect {
                    channel_handles,
                    file_handles,
                    socket_handles,
                    deadline,
                } => {
                    // Select: algum canal tem dado, algum file tem dado, algum socket
                    // tem dado, OU deadline expirou.
                    let channel_ready = channel_handles.iter().any(|h| can_recv(*h));
                    let file_ready = if file_handles.is_empty() {
                        false
                    } else {
                        try_select_files(file_handles) != FILE_WOULD_BLOCK
                    };
                    let socket_ready = if socket_handles.is_empty() {
                        false
                    } else {
                        try_select_sockets(socket_handles) != SOCKET_WOULD_BLOCK
                    };
                    if channel_ready || file_ready || socket_ready {
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
                BlockReason::WaitingOnSleep(deadline) => {
                    if std::time::Instant::now() >= *deadline {
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
    fn drain_pending_spawns(&mut self, arenas: &mut Vec<ArenaKind>, parent_id: FiberId) {
        let pending: Vec<(i64, i64, i64, i64)> = std::mem::take(&mut self.pending_spawns);
        for (fn_ptr, rt, caller_arena, args_ptr) in pending {
            // Usar parent_id como pai, não current_fiber (que é None
            // neste ponto).
            self.current_fiber = Some(parent_id);
            if let Err(e) = self.spawn(arenas, fn_ptr, rt, caller_arena, args_ptr) {
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
    fn try_destroy(&mut self, arenas: &mut Vec<ArenaKind>, fiber_id: FiberId) {
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

        // Fase 9: fechar arquivos abertos pelo fiber que não foram fechados
        // explicitamente com close!(). Isto previne FD leak quando um fiber
        // termina sem fechar arquivos alocados na sua arena.
        for handle in &entry.open_files {
            // SAFETY: handle é um FileInner válido alocado na fiber_arena.
            // kata_rt_file_close é idempotente (no-op se já fechado).
            unsafe { crate::file::kata_rt_file_close(*handle) };
        }
        entry.open_files.clear();

        // Destruir a arena do fiber via pool direto.
        if let Some(a) = arenas.get_mut(arena_handle as usize) {
            match a {
                ArenaKind::Bump(b) => b.reset(),
                ArenaKind::Tracked(t) => t.destroy(),
            }
        }
        // Drop manual do fiber.
        // SAFETY: fiber completou (completed=true), não está suspenso.
        unsafe {
            std::mem::ManuallyDrop::drop(&mut entry.fiber);
        }

        // Remover este fiber da lista de children do pai e propagar.
        if let Some(pid) = parent_id
            && let Some(parent) = self.fibers.get_mut(&pid)
        {
            parent.children.retain(|&c| c != fiber_id);
            self.try_destroy(arenas, pid);
        }
        // root_arena é destruída pelo `Drop` do Runtime.
    }
}

// Default removido — Scheduler::new() agora requer root_arena_handle.
