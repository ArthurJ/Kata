//! Marshalling — `to_bytes` / `from_bytes` para `spawn!` e IPC cross-process.
//!
//! `to_bytes` serializa um valor runtime em um blob `Bytes` com header estendido
//! (type_id + rebase_offsets), reaproveitando a mecânica de `HeapSnapshotData`
//! (Fio 12). `from_bytes` reconstrói o valor na arena destino.
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

use std::cell::RefCell;

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

thread_local! {
    static TYPE_TABLE: RefCell<Vec<TypeShape>> = const { RefCell::new(Vec::new()) };
}

/// Registra a type table Rust-to-Rust. Chamado pelo driver antes do JIT.
pub fn register_type_table(types: Vec<TypeShape>) {
    TYPE_TABLE.with(|table| {
        *table.borrow_mut() = types;
    });
}

/// Reseta a type table — chamado entre execuções de teste.
#[allow(dead_code)] // reservada para futura uso na infra IPC
pub(crate) fn reset_type_table() {
    TYPE_TABLE.with(|table| {
        table.borrow_mut().clear();
    });
}

fn get_type_shape(type_id: i64) -> Option<TypeShape> {
    TYPE_TABLE.with(|table| {
        let table = table.borrow();
        table.get(type_id as usize).cloned()
    })
}

/// Lê um i64 não-alinhado em `ptr + offset`. Usado por serialize e deserialize.
unsafe fn read_i64_at(ptr: *const u8, offset: usize) -> i64 {
    unsafe { std::ptr::read_unaligned(ptr.add(offset) as *const i64) }
}

// ════════════════════════════════════════════════════════════
//  Testes unitários
// ════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{kata_rt_arena_alloc, kata_rt_arena_create, kata_rt_arena_destroy};

    fn make_arena() -> i64 {
        kata_rt_arena_create()
    }

    fn register_types() {
        register_type_table(vec![
            TypeShape::Prim, // 0: Int
            TypeShape::Text, // 1: Text
            TypeShape::Tuple(vec![
                // 2: (Int, Text)
                TypeShape::Prim,
                TypeShape::Text,
            ]),
            TypeShape::List(Box::new(TypeShape::Prim)), // 3: List<Int>
        ]);
    }

    #[test]
    fn to_bytes_from_bytes_int() {
        register_types();
        let arena = make_arena();
        // Int 42 como SMI: (42 << 1) | 1 = 85
        let val = (42i64 << 1) | 1;
        let blob = kata_rt_to_bytes(val, 0, arena); // type_id=0 (Prim)
        assert!(blob != 0, "to_bytes produziu blob válido");
        let recovered = kata_rt_from_bytes(blob, arena);
        assert_eq!(recovered, val, "roundtrip Int SMI");
        kata_rt_arena_destroy(arena);
    }

    #[test]
    fn to_bytes_from_bytes_text() {
        register_types();
        let arena = make_arena();
        // Aloca "hello" como C string na arena.
        let s = b"hello\0";
        let text_ptr = kata_rt_arena_alloc(arena, s.len() as i64);
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), text_ptr as *mut u8, s.len());
        }
        let blob = kata_rt_to_bytes(text_ptr, 1, arena); // type_id=1 (Text)
        assert!(blob != 0, "to_bytes produziu blob válido");
        let recovered = kata_rt_from_bytes(blob, arena);
        assert!(recovered != 0, "from_bytes produziu ponteiro válido");
        // Verifica conteúdo.
        let cstr = unsafe { std::ffi::CStr::from_ptr(recovered as *const std::os::raw::c_char) };
        assert_eq!(cstr.to_bytes(), b"hello", "roundtrip Text");
        kata_rt_arena_destroy(arena);
    }

    #[test]
    fn to_bytes_from_bytes_tuple() {
        register_types();
        let arena = make_arena();
        // Tupla (42, "oi"):
        let tuple_ptr = kata_rt_arena_alloc(arena, 16);
        let s = b"oi\0";
        let text_ptr = kata_rt_arena_alloc(arena, s.len() as i64);
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), text_ptr as *mut u8, s.len());
            let smi = (42i64 << 1) | 1;
            std::ptr::write_unaligned(tuple_ptr as *mut i64, smi);
            std::ptr::write_unaligned((tuple_ptr as *mut u8).add(8) as *mut i64, text_ptr);
        }
        let blob = kata_rt_to_bytes(tuple_ptr, 2, arena); // type_id=2 (Tuple)
        assert!(blob != 0);
        let recovered = kata_rt_from_bytes(blob, arena);
        assert!(recovered != 0);
        // Lê campo 0 (Int SMI) e campo 1 (Text ptr).
        let field0 = unsafe { read_i64_at(recovered as *const u8, 0) };
        let field1 = unsafe { read_i64_at(recovered as *const u8, 8) };
        assert_eq!(field0, (42i64 << 1) | 1, "campo 0 = Int 42 SMI");
        assert!(field1 != 0, "campo 1 = Text ptr válido");
        let cstr = unsafe { std::ffi::CStr::from_ptr(field1 as *const std::os::raw::c_char) };
        assert_eq!(cstr.to_bytes(), b"oi", "campo 1 = \"oi\"");
        kata_rt_arena_destroy(arena);
    }

    #[test]
    fn to_bytes_from_bytes_list_int() {
        register_types();
        let arena = make_arena();
        // Lista [1, 2, 3] de Ints SMI.
        let mut list = 0i64;
        for &v in &[3i64, 2, 1] {
            let smi = (v << 1) | 1;
            list = crate::list::kata_rt_list_cons(smi, list, arena);
        }
        let blob = kata_rt_to_bytes(list, 3, arena); // type_id=3 (List<Int>)
        assert!(blob != 0);
        let recovered = kata_rt_from_bytes(blob, arena);
        assert!(recovered != 0);
        // Verifica: head=1, tail.head=2, tail.tail.head=3
        let h1 = crate::list::kata_rt_list_head(recovered);
        let t1 = crate::list::kata_rt_list_tail(recovered);
        let h2 = crate::list::kata_rt_list_head(t1);
        let t2 = crate::list::kata_rt_list_tail(t1);
        let h3 = crate::list::kata_rt_list_head(t2);
        assert_eq!(h1 >> 1, 1, "head = 1");
        assert_eq!(h2 >> 1, 2, "second = 2");
        assert_eq!(h3 >> 1, 3, "third = 3");
        kata_rt_arena_destroy(arena);
    }
}
