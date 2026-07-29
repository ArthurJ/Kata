//! `HeapSnapshotData` — dados serializados de um valor comptime para embed no binário.
//!
//! Definido em `kata-core` (não `kata-comptime`) para evitar dependência circular:
//! `TypedModule` (em `kata-inference`) precisa de `Vec<HeapSnapshotData>`,
//! e `kata-comptime` depende de `kata-inference`. Colocar em `kata-core` quebra o ciclo.
//!
//! A função `serialize_snapshot` (que produz `HeapSnapshotData`) vive em `kata-comptime/src/snapshot.rs`.
//! O runtime (`kata-rt`) consome `HeapSnapshotData` em load-time via `kata_rt_load_snapshots`.

use crate::ty::Ty;

/// Dados de um snapshot — bytes contíguos + offsets para rebasing.
///
/// O `bytes` é um buffer contíguo onde o valor serializado vive.
/// `rebase_offsets` lista as posições dentro de `bytes` que contêm
/// ponteiros (como offsets relativos). Em load-time, o runtime:
/// 1. `kata_rt_arena_alloc(root_arena, bytes.len())` → base_ptr
/// 2. `memcpy(base_ptr, bytes, bytes.len())`
/// 3. Para cada offset em `rebase_offsets`: `*(base_ptr + offset) += base_ptr`
///
/// O resultado é um ponteiro válido na root_arena com a mesma estrutura
/// do valor original.
#[derive(Debug, Clone)]
pub struct HeapSnapshotData {
    /// Bytes contíguos do valor serializado.
    pub bytes: Vec<u8>,
    /// Offsets dentro de `bytes` onde há ponteiros que precisam rebasing.
    /// Cada offset aponta para um i64 que é um offset relativo dentro do
    /// próprio buffer. Em load-time, soma-se `base_ptr` a cada um.
    pub rebase_offsets: Vec<usize>,
    /// Tipo do valor — para verificação de consistência.
    pub ty: Ty,
}
