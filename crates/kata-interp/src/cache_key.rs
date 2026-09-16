//! @cache: serialização de key por conteúdo.
//!
//! Espelha `kata_rt_serialize_key` do runtime (cache.rs) — mesmo formato
//! de bytes para que interp e JIT produzam keys idênticas para o mesmo valor.

use kata_core::StructKey;
use kata_core::ty::{PrimTy, Ty};

use crate::value::Value;

/// FNV-1a — mesmo hash do `canonical_fn_id` do codegen.
pub(crate) fn fnv1a(bytes: &[u8]) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

/// Serializa um argumento da cache key por CONTEÚDO (não por ponteiro).
///
/// Espelha `kata_rt_serialize_key` do runtime (cache.rs) — mesmo formato
/// de bytes para que interp e JIT produzam keys idênticas para o mesmo valor.
///
/// - Int/Rational: 8 bytes LE do valor
/// - Float: 8 bytes dos bits do f64
/// - Text: len (4 bytes LE) + bytes do C-string
/// - List: percorre cons cells, serializa cada head com o tipo do elemento
/// - Tuple: lê cada elemento do bloco contíguo (8 bytes cada)
/// - Struct: consulta struct_registry para campos, lê cada campo (8 bytes)
/// - Array: len (do header) + cada elemento via array_get
/// - Sum: tag (8 bytes) + payload (8 bytes crus — limitação mesma do JIT)
/// - Set/Dict: não serializável recursivamente → miss conservador
/// - Unit: 0 bytes
pub(crate) fn serialize_key_part(
    ty: &Ty,
    val: Value,
    key: &mut Vec<u8>,
    cacheable: &mut bool,
    struct_registry: &kata_core::StructRegistry,
) {
    match ty {
        Ty::Prim(PrimTy::Int) | Ty::Prim(PrimTy::Rational) => {
            key.extend_from_slice(&val.to_le_bytes());
        }
        Ty::Prim(PrimTy::Float) => {
            key.extend_from_slice(&val.to_le_bytes());
        }
        Ty::Prim(PrimTy::Text) => {
            if val == 0 {
                key.extend_from_slice(&0u32.to_le_bytes());
                return;
            }
            let cstr = unsafe { std::ffi::CStr::from_ptr(val as *const _) };
            let bytes = cstr.to_bytes();
            key.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            key.extend_from_slice(bytes);
        }
        Ty::Unit => {
            // Unit: 0 bytes — nada a escrever (mesmo do runtime TD_UNIT)
        }
        Ty::List(elem_ty) => {
            let mut current = val;
            while current != 0 {
                let ptr = current as *const u8;
                let head = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
                let tail = unsafe { std::ptr::read_unaligned(ptr.add(8) as *const i64) };
                serialize_key_part(elem_ty, head, key, cacheable, struct_registry);
                if !*cacheable {
                    return;
                }
                current = tail;
            }
            // Marcador de fim de lista (distingue [1] de [1 1])
            key.push(0);
        }
        Ty::Tuple(elem_tys) => {
            let ptr = val as *const u8;
            for (i, elem_ty) in elem_tys.iter().enumerate() {
                let elem_val = unsafe { std::ptr::read_unaligned(ptr.add(i * 8) as *const i64) };
                serialize_key_part(elem_ty, elem_val, key, cacheable, struct_registry);
                if !*cacheable {
                    return;
                }
            }
        }
        Ty::Struct(struct_key) => {
            // Refined sem campos delega ao tipo base (mesmo do codegen
            // write_descriptor — Instance resolve para o concreto)
            let type_name = struct_key.name();
            if let Some(info) = struct_registry.get(type_name) {
                if info.alias_of.is_some() && info.predicates.is_some() && info.fields.is_empty() {
                    let base = info.alias_of.clone().expect("checado is_some acima");
                    let base_ty = match base.as_str() {
                        "Int" => Ty::Prim(PrimTy::Int),
                        "Float" => Ty::Prim(PrimTy::Float),
                        "Text" => Ty::Prim(PrimTy::Text),
                        "Rational" => Ty::Prim(PrimTy::Rational),
                        _ => Ty::Struct(StructKey::Plain(base.clone())),
                    };
                    serialize_key_part(&base_ty, val, key, cacheable, struct_registry);
                    return;
                }
                // Struct com campos: ler cada campo (8 bytes contíguos)
                let ptr = val as *const u8;
                for (i, field) in info.fields.iter().enumerate() {
                    let field_val =
                        unsafe { std::ptr::read_unaligned(ptr.add(i * 8) as *const i64) };
                    serialize_key_part(&field.ty, field_val, key, cacheable, struct_registry);
                    if !*cacheable {
                        return;
                    }
                }
            } else {
                // Struct desconhecido no registry — não pode serializar
                *cacheable = false;
            }
        }
        Ty::Array(elem_ty) => {
            if val == 0 {
                // Array nulo = vazio
                key.extend_from_slice(&0u32.to_le_bytes());
                return;
            }
            let len_raw = unsafe { std::ptr::read_unaligned(val as *const i64) };
            let len = len_raw as usize;
            key.extend_from_slice(&(len as u32).to_le_bytes());
            for i in 0..len {
                let offset = 8 + i * 8;
                let elem_val = unsafe {
                    std::ptr::read_unaligned((val as *const u8).add(offset) as *const i64)
                };
                serialize_key_part(elem_ty, elem_val, key, cacheable, struct_registry);
                if !*cacheable {
                    return;
                }
            }
        }
        Ty::Sum(_) | Ty::Generic(_, _) => {
            // Sum: tag (8 bytes offset 0) + payload (8 bytes offset 8).
            // Limitação mesma do JIT: payload não serializado recursivamente
            // sem enum_registry. Dois Sums com mesmo payload em endereços
            // diferentes terão keys diferentes — aceitável por enquanto.
            if val == 0 {
                *cacheable = false;
                return;
            }
            let ptr = val as *const u8;
            let tag = unsafe { std::ptr::read_unaligned(ptr as *const i64) };
            let payload = unsafe { std::ptr::read_unaligned(ptr.add(8) as *const i64) };
            key.extend_from_slice(&tag.to_le_bytes());
            key.extend_from_slice(&payload.to_le_bytes());
        }
        // Set, Dict, Byte, Bytes, File, Interface, Var, Arrow, etc:
        // não serializáveis recursivamente → miss conservador
        _ => *cacheable = false,
    }
}
