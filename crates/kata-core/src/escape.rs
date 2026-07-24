//! EscapeTarget — destino de escape de um valor alocado.
//!
//! Generaliza `tail_pos: bool` para seleção de arena. O typeck determina
//! para onde um valor escapa (local ou caller) e o codegen usa essa
//! informação para escolher a arena de alocação.
//!
//! Coexiste com `tail_pos: bool` — `tail_pos` governa TCO (fluxo de controle),
//! `EscapeTarget` governa arena selection (memória).

/// Destino de escape de um valor na TAST.
///
/// Determina em qual arena o codegen aloca o valor:
/// - `Local` → `fiber_arena` (liberada quando o fiber termina)
/// - `Caller` → `caller_arena` (arena do pai direto)
/// - `Heap` → `root_arena` (TrackedArena — sobrevive a todos os fibers,
///   dealloc individual quando ARC refcount → 0)
///
/// Pré- (sem canais), os únicos casos são:
/// - Função pura / entry point → `Caller` (sem fiber_arena, usa caller_arena)
/// - Retorno de Action → `Caller`
/// - Computação local em Action → `Local`
///
/// Com canais (`!>`), valores enviados para outro fiber usam `Heap`:
/// - O valor sobrevive ao fiber sender
/// - O receiver recebe um ponteiro válido na root_arena
/// - Quando o receiver consume e o refcount → 0, a root_arena dealloca
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeTarget {
    /// Valor local ao fiber — aloca em `fiber_arena`.
    Local,
    /// Valor escapa para o caller — aloca em `caller_arena`.
    Caller,
    /// Valor escapa para outro fiber via canal — aloca em `root_arena`.
    Heap,
}
