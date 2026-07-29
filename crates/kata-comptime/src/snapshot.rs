//! Serialização de valores comptime para `HeapSnapshotData`.
//!
//! O comptime pass JIT-executa expressões `@comptime` que produzem tipos
//! complexos (List, Struct, Tuple, Text, Sum com payload). O resultado é
//! um ponteiro i64 para dados na arena temporária do comptime. Este módulo
//! serializa esses dados em bytes contíguos + rebase_offsets, permitindo
//! que o runtime os recrie em load-time na root_arena.
//!
//! `HeapSnapshotData` é definido em `kata-core` para evitar dependência circular.

use kata_core::StructRegistry;
use kata_core::snapshot::HeapSnapshotData;
use kata_core::ty::{PrimTy, Ty};

/// Serializa um valor JIT-executado em `HeapSnapshotData`.
///
/// `raw` é o valor i64 retornado pelo JIT (ponteiro para tipos complexos).
/// `ty` é o tipo canónico do valor.
///
/// Para tipos complexos, `raw` é um ponteiro absoluto para dados na arena
/// temporária do comptime. A serialização caminha a estrutura, copiando
/// valores e convertendo ponteiros em offsets relativos dentro do buffer.
pub fn serialize_snapshot(
    raw: i64,
    ty: &Ty,
    struct_registry: &StructRegistry,
) -> Result<HeapSnapshotData, String> {
    let mut ser = Serializer::new();
    serialize_value(&mut ser, raw, ty, struct_registry)?;
    Ok(HeapSnapshotData {
        bytes: ser.bytes,
        rebase_offsets: ser.rebase_offsets,
        ty: ty.clone(),
    })
}

/// Serializador — acumula bytes e rebase_offsets.
struct Serializer {
    bytes: Vec<u8>,
    rebase_offsets: Vec<usize>,
}

impl Serializer {
    fn new() -> Self {
        Serializer {
            bytes: Vec::new(),
            rebase_offsets: Vec::new(),
        }
    }

    /// Alinha para 8 bytes.
    fn align8(&mut self) {
        while self.bytes.len() % 8 != 0 {
            self.bytes.push(0);
        }
    }

    /// Escreve um i64 no buffer (alinhado a 8).
    fn write_i64(&mut self, val: i64) {
        self.align8();
        self.bytes.extend_from_slice(&val.to_le_bytes());
    }

    /// Escreve um ponteiro como offset relativo + regista para rebasing.
    /// `relative_offset` é o offset dentro do buffer que o ponteiro aponta.
    /// Se `relative_offset` == 0, é Nil/null — não precisa rebasing.
    fn write_ptr_offset(&mut self, relative_offset: i64) {
        self.align8();
        let pos = self.bytes.len();
        self.bytes.extend_from_slice(&relative_offset.to_le_bytes());
        if relative_offset != 0 {
            self.rebase_offsets.push(pos);
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// Posição actual alinhada a 8 bytes (onde o próximo write_i64 iria).
    fn aligned_len(&self) -> usize {
        let len = self.bytes.len();
        if len % 8 == 0 {
            len
        } else {
            len + (8 - len % 8)
        }
    }
}

/// Lê um i64 de um endereço absoluto.
unsafe fn read_i64_at(ptr: *const u8, offset: usize) -> i64 {
    unsafe { std::ptr::read_unaligned(ptr.add(offset) as *const i64) }
}

/// Serializa um valor recursivamente.
///
/// `raw` é o valor i64:
/// - Para escalares (Int SMI, Float, Boolean, Unit): valor imediato.
/// - Para tipos complexos (List, Struct, Tuple, Text, Sum): ponteiro absoluto
///   para dados na arena temporária.
fn serialize_value(
    ser: &mut Serializer,
    raw: i64,
    ty: &Ty,
    struct_registry: &StructRegistry,
) -> Result<(), String> {
    match ty {
        Ty::List(elem_ty) => {
            serialize_list(ser, raw, elem_ty, struct_registry)?;
        }
        Ty::Tuple(elements) => {
            let ptr = raw as *const u8;
            for (i, elem_ty) in elements.iter().enumerate() {
                let word = unsafe { read_i64_at(ptr, i * 8) };
                serialize_value(ser, word, elem_ty, struct_registry)?;
            }
        }
        Ty::Struct(name) => {
            if let Some(info) = struct_registry.get(name) {
                let ptr = raw as *const u8;
                for (i, field) in info.fields.iter().enumerate() {
                    let word = unsafe { read_i64_at(ptr, i * 8) };
                    serialize_value(ser, word, &field.ty, struct_registry)?;
                }
            } else {
                ser.write_i64(raw);
            }
        }
        Ty::Sum(name) => {
            if name == "Boolean" {
                ser.write_i64(raw);
            } else {
                // Sum com payload: tag (offset 0) + payload (offset 8) = 16 bytes.
                let ptr = raw as *const u8;
                let tag = unsafe { read_i64_at(ptr, 0) };
                let payload = unsafe { read_i64_at(ptr, 8) };
                ser.write_i64(tag);
                ser.write_i64(payload);
            }
        }
        Ty::Prim(PrimTy::Text) => {
            if raw == 0 {
                ser.write_i64(0);
            } else {
                // Text é C string (nulo-terminada).
                let ptr = raw as *const std::os::raw::c_char;
                let str_bytes = unsafe { std::ffi::CStr::from_ptr(ptr).to_bytes() };
                // Layout: i64 offset (relativo) + bytes da string + nul.
                // O offset aponta para os bytes da string (após o i64).
                let str_start = ser.aligned_len() + 8;
                ser.write_ptr_offset(str_start as i64);
                ser.write_bytes(str_bytes);
                ser.write_bytes(&[0u8]); // nul terminator
            }
        }
        Ty::Prim(PrimTy::Int) => {
            // SMI (LSB=1) é valor imediato. BigInt (LSB=0) é ponteiro.
            // BigInt serialização fica para depois — por ora escreve o raw.
            ser.write_i64(raw);
        }
        Ty::Prim(PrimTy::Float) => {
            ser.write_i64(raw);
        }
        Ty::Unit => {
            ser.write_i64(0);
        }
        _ => {
            ser.write_i64(raw);
        }
    }
    Ok(())
}

/// Serializa uma lista Cons (head: i64, tail: ptr|0).
///
/// Cada Cons cell é 16 bytes: head (i64) + tail (i64).
/// Percorre a lista, escrevendo cada cell no buffer. O `tail` de cada
/// cell é convertido para offset relativo da próxima cell (ou 0 se Nil).
///
/// Para elementos escalares (Int, Float, Boolean), cada cell ocupa
/// exactamente 16 bytes. Para elementos complexos (Text, nested List),
/// o head pode ter tamanho variável — o offset do tail é calculado
/// dinamicamente.
fn serialize_list(
    ser: &mut Serializer,
    ptr: i64,
    elem_ty: &Ty,
    struct_registry: &StructRegistry,
) -> Result<(), String> {
    let mut current = ptr;
    while current != 0 {
        let raw_ptr = current as *const u8;
        let head = unsafe { read_i64_at(raw_ptr, 0) };
        let tail = unsafe { read_i64_at(raw_ptr, 8) };

        // Escreve head.
        serialize_value(ser, head, elem_ty, struct_registry)?;

        // Escreve tail como offset relativo ou 0.
        if tail == 0 {
            ser.write_i64(0);
        } else {
            // O tail aponta para a próxima cell, que será escrita a seguir.
            // O offset relativo é a posição actual do buffer (alinhada)
            // após escrever este i64.
            let next_cell_offset = ser.aligned_len() + 8; // +8 por este i64
            ser.write_ptr_offset(next_cell_offset as i64);
        }

        current = tail;
    }
    Ok(())
}
