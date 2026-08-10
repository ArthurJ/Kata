//! Serialização de valores comptime para `HeapSnapshotData`.
//!
//! Comptime pass JIT-executa expressões `@comptime` que produzem tipos
//! complexos (List, Struct, Tuple, Text, Sum com payload). O resultado é
//! um ponteiro i64 para dados na arena temporária do comptime. Este módulo
//! serializa esses dados em bytes contíguos + rebase_offsets, permitindo
//! que o runtime os recrie em load-time na root_arena.
//!
//! `HeapSnapshotData` é definido em `kata-core` para evitar dependência circular.
//!
//! ## Layout (Opção C — tudo ocupa 8 bytes)
//!
//! O buffer tem duas regiões:
//! - **main:** campos/cells de 8 bytes cada. O `FieldAccess`/`IndexAccess`
//!   faz `load(ptr + i*8)` e sempre acerta o campo.
//! - **appended:** strings (C strings nulo-terminadas) e dados variáveis.
//!
//! Cada i64 na main que é ponteiro (Text, List, nested Struct) é um offset
//! relativo para dados na appended. Esses offsets são registrados em
//! `rebase_offsets` para rebasing (soma `base_ptr` no load-time).
//!
//! Para Text standalone, o snapshot é um único i64 (offset para a string
//! na appended). O codegen precisa fazer `load(ptr + 0)` para obter o
//! ponteiro da string, ou o lowering precisa somar o offset do header.

use kata_core::EnumRegistry;
use kata_core::StructRegistry;
use kata_core::snapshot::HeapSnapshotData;
use kata_core::ty::{PrimTy, Ty};

/// Serializa um valor JIT-executado em `HeapSnapshotData`.
///
/// `raw` é o valor i64 retornado pelo JIT (ponteiro para tipos complexos).
/// `ty` é o tipo canônico do valor.
///
/// Para tipos complexos, `raw` é um ponteiro absoluto para dados na arena
/// temporária do comptime. A serialização caminha a estrutura, copiando
/// valores e convertendo ponteiros em offsets relativos dentro do buffer.
pub(crate) fn serialize_snapshot(
    raw: i64,
    ty: &Ty,
    struct_registry: &StructRegistry,
    enum_registry: &EnumRegistry,
) -> Result<HeapSnapshotData, String> {
    let mut ser = Serializer::new();
    serialize_value(&mut ser, raw, ty, struct_registry, enum_registry)?;
    // finish() consome ser e ajusta os appended_rebase_offsets somando
    // main_len, depois mescla com rebase_offsets. Retorna o buffer final.
    let bytes = ser.finish();
    // Após finish(), ser.rebase_offsets foi atualizado com os appended offsets.
    // Mas ser foi consumido — precisamos reorganizar.
    Ok(HeapSnapshotData {
        bytes: bytes.0,
        rebase_offsets: bytes.1,
        ty: ty.clone(),
    })
}

/// Serializador — acumula bytes em duas regiões (main + appended).
///
/// main: campos/cells de 8 bytes. Ponteiros são offsets relativos para
///   dados na appended.
/// appended: strings e dados variáveis. Offset absoluto dentro do buffer
///   final (main ++ appended) é calculado no `finish()`.
struct Serializer {
    /// Região principal — campos/cells de 8 bytes cada.
    main: Vec<u8>,
    /// Região appended — strings e dados variáveis.
    appended: Vec<u8>,
    /// Offsets na main onde há ponteiros relativos para a main (List tail).
    /// O i64 armazena o offset dentro da main. Rebasing: +base_ptr.
    rebase_offsets: Vec<usize>,
    /// Offsets na main onde há ponteiros para a appended (Text, etc.).
    /// O i64 armazena o offset dentro da appended. Rebasing: +base_ptr+main_len.
    /// Precisamos ajustar no finish() somando main_len ao i64.
    appended_rebase_offsets: Vec<usize>,
}

impl Serializer {
    fn new() -> Self {
        Serializer {
            main: Vec::new(),
            appended: Vec::new(),
            rebase_offsets: Vec::new(),
            appended_rebase_offsets: Vec::new(),
        }
    }

    /// Alinha a região main para 8 bytes.
    fn align_main(&mut self) {
        while !self.main.len().is_multiple_of(8) {
            self.main.push(0);
        }
    }

    /// Escreve um i64 na região main (alinhado a 8).
    fn write_i64(&mut self, val: i64) {
        self.align_main();
        self.main.extend_from_slice(&val.to_le_bytes());
    }

    /// Escreve um ponteiro para a main (offset relativo na própria main).
    /// Usado para List tail → próxima cell.
    /// `main_offset` é o offset dentro da main. Se 0, é Nil/null.
    fn write_main_ptr(&mut self, main_offset: i64) {
        self.align_main();
        let pos = self.main.len();
        self.main.extend_from_slice(&main_offset.to_le_bytes());
        if main_offset != 0 {
            self.rebase_offsets.push(pos);
        }
    }

    /// Escreve um ponteiro para a appended (offset relativo para a appended).
    /// Usado para Text → string na appended.
    /// `appended_offset` é o offset dentro da appended.
    fn write_appended_ptr(&mut self, appended_offset: usize) {
        self.align_main();
        let pos = self.main.len();
        // O i64 armazena o offset dentro da appended. No finish(),
        // somamos main_len para obter o offset absoluto no buffer.
        self.main
            .extend_from_slice(&(appended_offset as i64).to_le_bytes());
        self.appended_rebase_offsets.push(pos);
    }

    /// Escreve nil (0) na main.
    fn write_nil(&mut self) {
        self.align_main();
        self.main.extend_from_slice(&0i64.to_le_bytes());
    }

    /// Escreve bytes na appended (strings, dados variáveis).
    /// Retorna o offset dentro da appended onde os bytes foram escritos.
    fn write_appended(&mut self, bytes: &[u8]) -> usize {
        let offset = self.appended.len();
        self.appended.extend_from_slice(bytes);
        offset
    }

    /// Posição atual alinhada na main (onde o próximo write_i64 iria).
    fn main_aligned_len(&self) -> usize {
        let len = self.main.len();
        if len.is_multiple_of(8) {
            len
        } else {
            len + (8 - len % 8)
        }
    }

    /// Concatena main ++ appended num único buffer.
    /// Ajusta os appended_rebase_offsets somando main_len ao i64.
    /// Retorna (bytes, rebase_offsets) — todos os offsets que precisam rebasing.
    fn finish(mut self) -> (Vec<u8>, Vec<usize>) {
        let main_len = self.main.len() as i64;
        // Para cada offset que aponta para a appended, somar main_len
        // ao i64 armazenado, convertendo-o de offset-da-appended para
        // offset-absoluto-no-buffer.
        for &pos in &self.appended_rebase_offsets {
            let ptr = self.main.as_mut_ptr();
            unsafe {
                let val_ptr = ptr.add(pos) as *mut i64;
                let val = std::ptr::read_unaligned(val_ptr);
                std::ptr::write_unaligned(val_ptr, val + main_len);
            }
        }
        // Mescla os offsets da appended com os da main.
        let mut all_offsets = self.rebase_offsets;
        all_offsets.extend_from_slice(&self.appended_rebase_offsets);

        let mut bytes = self.main;
        bytes.extend_from_slice(&self.appended);
        (bytes, all_offsets)
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
///
/// Cada valor ocupa exatamente 8 bytes na main. Text e dados variáveis
/// vão para a appended, com um ponteiro (offset relativo) na main.
fn serialize_value(
    ser: &mut Serializer,
    raw: i64,
    ty: &Ty,
    struct_registry: &StructRegistry,
    enum_registry: &EnumRegistry,
) -> Result<(), String> {
    match ty {
        Ty::List(elem_ty) => {
            serialize_list(ser, raw, elem_ty, struct_registry, enum_registry)?;
        }
        Ty::Tuple(elements) => {
            let ptr = raw as *const u8;
            for (i, elem_ty) in elements.iter().enumerate() {
                let word = unsafe { read_i64_at(ptr, i * 8) };
                serialize_value(ser, word, elem_ty, struct_registry, enum_registry)?;
            }
        }
        Ty::Struct(name) => {
            if let Some(info) = struct_registry.get(name) {
                let ptr = raw as *const u8;
                for (i, field) in info.fields.iter().enumerate() {
                    let word = unsafe { read_i64_at(ptr, i * 8) };
                    serialize_value(ser, word, &field.ty, struct_registry, enum_registry)?;
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
                // Serializar o payload recursivamente. Precisamos mapear
                // tag → variant → payload_ty. Para Sum não-genérico, o
                // payload_ty é fixo por variant (sem type_args).
                let payload_ty = resolve_payload_ty(enum_registry, name, tag as usize);
                match payload_ty {
                    Some(pty) => {
                        serialize_value(ser, payload, &pty, struct_registry, enum_registry)?;
                    }
                    None => {
                        // Variante sem payload ou enum não registrado —
                        // copia o i64 cru (ex: Boolean já tratado acima).
                        ser.write_i64(payload);
                    }
                }
            }
        }
        Ty::Generic(name, type_args) => {
            // Generic Sum (ex: Result<Int, Text>) — mesmo layout que Sum:
            // tag (offset 0) + payload (offset 8).
            if name == "Boolean" {
                ser.write_i64(raw);
            } else {
                let ptr = raw as *const u8;
                let tag = unsafe { read_i64_at(ptr, 0) };
                let payload = unsafe { read_i64_at(ptr, 8) };
                ser.write_i64(tag);
                // Serializar o payload recursivamente, instanciando o
                // payload_ty com os type_args concretos.
                let payload_ty =
                    resolve_generic_payload_ty(enum_registry, name, tag as usize, type_args);
                match payload_ty {
                    Some(pty) => {
                        serialize_value(ser, payload, &pty, struct_registry, enum_registry)?;
                    }
                    None => {
                        // Variante sem payload ou enum não registrado.
                        ser.write_i64(payload);
                    }
                }
            }
        }
        Ty::Prim(PrimTy::Text) => {
            if raw == 0 {
                ser.write_nil();
            } else {
                // Text é C string (nulo-terminada). Escreve a string na appended
                // e um ponteiro (offset relativo) na main.
                let ptr = raw as *const std::os::raw::c_char;
                let str_bytes = unsafe { std::ffi::CStr::from_ptr(ptr).to_bytes() };
                let mut bytes_with_nul = Vec::with_capacity(str_bytes.len() + 1);
                bytes_with_nul.extend_from_slice(str_bytes);
                bytes_with_nul.push(0u8); // nul terminator
                let appended_offset = ser.write_appended(&bytes_with_nul);
                ser.write_appended_ptr(appended_offset);
            }
        }
        Ty::Prim(PrimTy::Int) => {
            // SMI (LSB=1) é valor imediato. BigInt (LSB=0) é ponteiro.
            ser.write_i64(raw);
        }
        Ty::Prim(PrimTy::Float) => {
            ser.write_i64(raw);
        }
        Ty::Unit => {
            ser.write_i64(0);
        }
        Ty::Function(_params, _ret) => {
            // Closure — CaptureBox na arena.
            // Layout: fn_ptr (offset 0), refcount (offset 8),
            // n_captures (offset 16), captures[0..n] (offset 24+).
            //
            // fn_ptr é ponteiro absoluto para código JIT (páginas leaked
            // permanecem mapeadas) — não precisa rebase.
            // captures são ou valores imediatos (SMI, Float) ou ponteiros
            // absolutos para dados na arena persistente. Como o Runtime do
            // REPL persiste entre linhas, os ponteiros originais permanecem
            // válidos. Serializamos como raw i64s sem rebase.
            if raw == 0 {
                ser.write_nil();
            } else {
                let ptr = raw as *const u8;
                let fn_ptr = unsafe { read_i64_at(ptr, 0) };
                let refcount = unsafe { read_i64_at(ptr, 8) };
                let n_captures = unsafe { read_i64_at(ptr, 16) };

                ser.write_i64(fn_ptr);
                ser.write_i64(refcount);
                ser.write_i64(n_captures);

                let n = n_captures as usize;
                for i in 0..n {
                    let cap = unsafe { read_i64_at(ptr, 24 + i * 8) };
                    ser.write_i64(cap);
                }
            }
        }
        _ => {
            ser.write_i64(raw);
        }
    }
    Ok(())
}

/// Serializa uma lista Cons (head: i64, tail: ptr|0).
///
/// Cada Cons cell é 16 bytes na main: head (i64) + tail (i64).
/// Percorre a lista, escrevendo cada cell. O `tail` de cada
/// cell é convertido para offset relativo da próxima cell (ou 0 se Nil).
///
/// Para elementos escalares (Int, Float, Boolean), cada head ocupa
/// exactamente 8 bytes. Para Text, o head é um ponteiro (offset relativo)
/// para a string na appended.
fn serialize_list(
    ser: &mut Serializer,
    ptr: i64,
    elem_ty: &Ty,
    struct_registry: &StructRegistry,
    enum_registry: &EnumRegistry,
) -> Result<(), String> {
    let mut current = ptr;
    while current != 0 {
        let raw_ptr = current as *const u8;
        let head = unsafe { read_i64_at(raw_ptr, 0) };
        let tail = unsafe { read_i64_at(raw_ptr, 8) };

        // Escreve head (8 bytes — valor escalar ou ponteiro para appended).
        serialize_value(ser, head, elem_ty, struct_registry, enum_registry)?;

        // Escreve tail como offset relativo ou 0.
        if tail == 0 {
            ser.write_nil();
        } else {
            // O tail aponta para a próxima cell, que será escrita a seguir.
            // O offset relativo é a posição actual da main (alinhada)
            // após escrever este i64.
            let next_cell_offset = ser.main_aligned_len() + 8; // +8 por este i64
            ser.write_main_ptr(next_cell_offset as i64);
        }

        current = tail;
    }
    Ok(())
}

/// Resolve o tipo do payload de uma variante de Sum não-genérico.
///
/// Mapeia tag (índice da variante) → nome da variante → payload_ty.
/// Retorna `None` se a variante não tem payload ou o enum não está registrado.
fn resolve_payload_ty(enum_registry: &EnumRegistry, enum_name: &str, tag: usize) -> Option<Ty> {
    let variants = enum_registry.all_variants(enum_name)?;
    let variant = variants.get(tag)?;
    variant.payload_ty.clone()
}

/// Resolve o tipo do payload de uma variante de Sum genérico, instanciando
/// com os type_args concretos.
///
/// Ex: `Result::Err "fail"` tem tag=1 (Err), type_args=[Int, Text].
/// O payload_ty de Err é `Ty::Var("E")`. Substituindo E→Text, obtemos
/// `Ty::Prim(PrimTy::Text)`.
fn resolve_generic_payload_ty(
    enum_registry: &EnumRegistry,
    enum_name: &str,
    tag: usize,
    type_args: &[Ty],
) -> Option<Ty> {
    let variants = enum_registry.all_variants(enum_name)?;
    let variant = variants.get(tag)?;
    let payload_ty = variant.payload_ty.as_ref()?;
    // Se o enum é genérico, precisamos instanciar o payload_ty com os
    // type_args concretos. instantiate_variant faz a substituição de
    // Ty::Var("T") → type_args[0], etc.
    if enum_registry.is_generic(enum_name) {
        enum_registry.instantiate_variant(enum_name, &variant.name, type_args)
    } else {
        Some(payload_ty.clone())
    }
}
