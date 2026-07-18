//! EscapeTarget — destino de escape de um valor alocado.
//!
//! Generaliza `tail_pos: bool` para seleção de arena. O typeck determina
//! para onde um valor escapa (local, caller, ancestral distante) e o codegen
//! usa essa informação para escolher a arena de alocação.
//!
//! Coexiste com `tail_pos: bool` — `tail_pos` governa TCO (fluxo de controle),
//! `EscapeTarget` governa arena selection (memória).

/// Destino de escape de um valor na TAST.
///
/// Determina em qual arena o codegen aloca o valor:
/// - `Local` → `fiber_arena` (liberada quando o fiber termina)
/// - `Caller` → `caller_arena` (arena do pai direto)
/// - `Ancestor(n)` → arena do ancestral n níveis acima (LCA)
///
/// Pré- (sem canais), os únicos casos são:
/// - Função pura / entry point → `Ancestor(0)` (raiz)
/// - Retorno de Action → `Caller`
/// - Computação local em Action → `Local`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeTarget {
    /// Valor local ao fiber — aloca em `fiber_arena`.
    Local,
    /// Valor escapa para o caller direto — aloca em `caller_arena`.
    Caller,
    /// Valor escapa para ancestral distante — aloca na arena do LCA.
    /// O índice é a profundidade do LCA relativa ao fiber atual.
    /// `Ancestor(0)` = raiz (arena do scheduler).
    Ancestor(u32),
}
