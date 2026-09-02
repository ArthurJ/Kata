//! Marshalling — `to_bytes` / `from_bytes` para `spawn!` e IPC cross-process.
//!
//! `to_bytes` serializa um valor runtime em um blob `Bytes` com header estendido
//! (type_id + rebase_offsets), reaproveitando a mecânica de `HeapSnapshotData`.
//! `from_bytes` reconstrói o valor na arena destino.
//!
//! ## Type table
//!
//! `TypeShape` é a projeção runtime de `Ty` — carrega apenas a informação
//! estrutural necessária para caminhar o valor (fields, variants, element
//! types). A type table é registrada Rust-to-Rust pelo driver (não via FFI
//! C-ABI) antes de executar o JIT, armazenada em TLS. As FFIs `kata_rt_to_bytes`
//! e `kata_rt_from_bytes` consultam a type table por `type_id`.
//!
//! ## Layout do blob
//!
//! ```text
//! Bytes header (8 bytes):
//!   offset 0:  content_len (i64) — tamanho do conteúdo (sem este header)
//!
//! Conteúdo (content_len bytes):
//!   offset 0:   data_len (i64) — tamanho dos dados serializados
//!   offset 8:   type_id (i64) — tipo do valor (índice na type table)
//!   offset 16:  rebase_count (i64) — número de offsets para rebasing
//!   offset 24:  rebase_offsets[rebase_count] (i64 cada)
//!   offset 24+rebase_count*8: data[0..data_len] — bytes serializados
//! ```
//!
//! ## Estrutura do módulo
//!
//! - `serialize`: lógica de serialização (`Serializer`, `serialize_value`,
//!   `kata_rt_to_bytes`).
//! - `deserialize`: lógica de desserialização (`Deserializer`,
//!   `deserialize_value`, `kata_rt_from_bytes`).
//! - Helpers compartilhados (`TypeShape`, type table, `read_i64_at`) ficam
//!   neste `mod.rs`.

mod deserialize;
mod serialize;

pub use deserialize::kata_rt_from_bytes;
pub use serialize::kata_rt_to_bytes;

/// Projeção runtime de `Ty` para marshalling — informação estrutural mínima
/// para caminhar um valor em runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeShape {
    /// Int, Float, Byte — 8 bytes inline (SMI-tagged ou raw bits).
    Prim,
    /// Unit — zero bytes.
    Unit,
    /// Text — C string (ponteiro na arena, nulo-terminada).
    Text,
    /// Bytes — blob contíguo (ponteiro na arena, layout 8+len).
    Bytes,
    /// Tupla — elementos heterogêneos, cada um 8 bytes.
    Tuple(Vec<TypeShape>),
    /// Struct — campos em ordem de declaração, cada um 8 bytes.
    Struct(Vec<TypeShape>),
    /// Sum (enum) — variantes com payload opcional.
    /// tag (i64) no offset 0, payload (i64) no offset 8.
    Sum(Vec<Option<Box<TypeShape>>>),
    /// List — Cons cells (head: 8 bytes, tail: ptr|0).
    List(Box<TypeShape>),
    /// Array — contíguo (len: i64, elements: i64 cada).
    Array(Box<TypeShape>),
}

/// Registra a type table no Runtime. Chamado pelo driver antes do JIT.
pub fn register_type_table(rt: i64, types: Vec<TypeShape>) {
    let runtime = unsafe { &mut *(rt as *mut crate::runtime::Runtime) };
    runtime.type_table = types;
}

fn get_type_shape(rt: i64, type_id: i64) -> Option<TypeShape> {
    let runtime = unsafe { &*(rt as *const crate::runtime::Runtime) };
    runtime.type_table.get(type_id as usize).cloned()
}

/// Lê um i64 não-alinhado em `ptr + offset`. Usado por serialize e deserialize.
unsafe fn read_i64_at(ptr: *const u8, offset: usize) -> i64 {
    unsafe { std::ptr::read_unaligned(ptr.add(offset) as *const i64) }
}

// ════════════════════════════════════════════════════════════
//  Testes unitários
// ════════════════════════════════════════════════════════════

#[cfg(test)]
#[path = "marshal_tests.rs"]
mod tests;
