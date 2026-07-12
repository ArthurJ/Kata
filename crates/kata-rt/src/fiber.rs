//! Fiber — wrapper sobre wasmtime-fiber para execução de Actions.
//!
//! Cada fiber tem sua própria stack (1 MB) e arena local. O scheduler cria
//! o fiber via `spawn`, passa `SpawnArgs` no `resume()`, e o trampoline
//! chama a função JIT com a assinatura uniforme `(fiber_arena, caller_arena, args_ptr) -> i64`.
//!
//! `Fiber<'static, SpawnArgs, (), i64>`:
//! - `Resume = SpawnArgs` — argumentos passados do host para o fiber
//! - `Yield = ()` — não usado em Fase 10 (sem canais)
//! - `Return = i64` — resultado da função JIT

use wasmtime_fiber::{Fiber, FiberStack};

/// Tamanho da stack de cada fiber.
const FIBER_STACK_SIZE: usize = 1024 * 1024; // 1 MB

/// Argumentos passados do scheduler para o fiber via `resume()`.
///
/// `#[repr(C)]` garante layout determinístico para a fronteira FFI.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpawnArgs {
    /// Ponteiro da função JIT a executar (transmutado para `extern "C" fn`).
    pub fn_ptr: i64,
    /// Arena do caller — onde alocar valores de retorno (sobrevivem ao fiber).
    pub caller_arena: i64,
    /// Ponteiro para a tupla de argumentos (0 se Unit).
    pub args_ptr: i64,
    /// Arena do fiber — criada pelo scheduler, destruída após o fiber retornar.
    pub fiber_arena: i64,
}

/// Trampoline genérico: recebe `SpawnArgs` e chama a função JIT.
///
/// A função JIT tem assinatura `extern "C" fn(i64, i64, i64) -> i64`:
/// `(fiber_arena, caller_arena, args_ptr) -> i64`.
fn trampoline(args: SpawnArgs, _suspend: &mut wasmtime_fiber::Suspend<SpawnArgs, (), i64>) -> i64 {
    let func: extern "C" fn(i64, i64, i64) -> i64 = unsafe { core::mem::transmute(args.fn_ptr) };
    func(args.fiber_arena, args.caller_arena, args.args_ptr)
}

/// Um fiber Kata com sua arena associada.
pub struct KataFiber {
    fiber: Fiber<'static, SpawnArgs, (), i64>,
    /// Handle da arena do fiber — destruída quando o fiber termina.
    pub arena_handle: i64,
}

impl KataFiber {
    /// Cria um novo fiber com stack de 1 MB e arena associada.
    ///
    /// O caller é responsável por criar a arena (`kata_rt_arena_create`) e
    /// passar o handle aqui. A arena é destruída pelo scheduler após `resume`.
    pub fn new(arena_handle: i64) -> Result<Self, String> {
        let stack = FiberStack::new(FIBER_STACK_SIZE, false)
            .map_err(|e| format!("failed to create fiber stack: {e}"))?;
        let fiber =
            Fiber::new(stack, trampoline).map_err(|e| format!("failed to create fiber: {e}"))?;
        Ok(KataFiber {
            fiber,
            arena_handle,
        })
    }

    /// Resume o fiber com os argumentos fornecidos.
    ///
    /// Retorna `Ok(i64)` se o fiber completou, `Err(())` se suspendeu.
    /// Em Fase 10, `yield` não é usado — o fiber sempre completa em 1 `resume`.
    #[allow(clippy::result_unit_err)] // () representa "suspended", não erro
    pub fn resume(&self, args: SpawnArgs) -> Result<i64, ()> {
        self.fiber.resume(args)
    }

    /// Retorna `true` se o fiber já completou.
    pub fn done(&self) -> bool {
        self.fiber.done()
    }
}
