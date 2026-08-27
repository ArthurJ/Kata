//! Show — formatação de valores compostos para display.
//!
//! O codegen sintetiza funções `__kata_show__Type` que o interpretador
//! não tem. Em vez de gerar código, o interpretador implementa show
//! diretamente aqui, usando o `Ty` do valor (disponível no `eval.rs`
//! no braço `Closure`).

use std::ffi::CString;

use kata_core::ty::{PrimTy, Ty};
use kata_rt as rt;

use crate::eval::InterpCtx;
use crate::value::{Value, decode_smi, is_smi, value_to_f64};

/// Formata um valor como Text (ponteiro C string), dado seu tipo.
///
/// Retorna um `i64` que é um `*mut c_char` (Text ptr no runtime).
pub fn show_value(val: Value, ty: &Ty, ctx: &InterpCtx) -> Value {
    match ty {
        // ── Primitivos ───────────────────────────────────────
        Ty::Prim(PrimTy::Int) => {
            if is_smi(val) {
                let n = decode_smi(val);
                rt::kata_rt_int_to_text(n) as i64
            } else {
                // BigInt pointer
                rt::kata_rt_bi_show(val) as i64
            }
        }
        Ty::Prim(PrimTy::Float) => {
            let f = value_to_f64(val);
            rt::kata_rt_float_to_text(f) as i64
        }
        Ty::Prim(PrimTy::Text) => {
            // show de Text cita com aspas: "texto"
            // kata_rt_bi_show faz isso para nós (BigInt/Text compartilham o show FFI)
            // Na verdade, show de Text envolve com aspas. O codegen usa kata_rt_bi_show
            // que apenas retorna o texto. Precisamos envolver com aspas manualmente.
            let raw = unsafe {
                std::ffi::CStr::from_ptr(val as *const std::os::raw::c_char)
                    .to_string_lossy()
                    .into_owned()
            };
            let quoted = format!("\"{raw}\"");
            CString::new(quoted)
                .unwrap_or_else(|_| CString::new("\"\"").unwrap())
                .into_raw() as i64
        }
        Ty::Prim(PrimTy::Rational) => {
            let r = val as *const num_rational::BigRational;
            unsafe { rt::kata_rt_rat_show(r) as i64 }
        }

        // ── Unit ──────────────────────────────────────────────
        Ty::Unit => CString::new("()").unwrap().into_raw() as i64,

        // ── Boolean — i64 cru (1=True, 0=False) ───────────────
        Ty::Sum(name) if name == "Boolean" => {
            if val == 1 {
                CString::new("True").unwrap().into_raw() as i64
            } else {
                CString::new("False").unwrap().into_raw() as i64
            }
        }

        // ── List [T] — Cons cells encadeadas ──────────────────
        Ty::List(elem_ty) => show_list(val, elem_ty, ctx),

        // ── Array {T} — bloco contíguo ────────────────────────
        Ty::Array(elem_ty) => show_array(val, elem_ty, ctx),

        // ── Tuple (A, B, ...) — ponteiro para N values ────────
        Ty::Tuple(elem_tys) => show_tuple(val, elem_tys, ctx),

        // ── Struct — ponteiro para N values com nomes ─────────
        Ty::Struct(struct_key) => show_struct(val, struct_key, ty, ctx),

        // ── Sum/Enum — tag + payload opcional ────────────────
        Ty::Sum(name) => show_sum(val, name, ty, ctx),

        // ── Tipos não suportados ainda ────────────────────────
        _ => {
            let placeholder = format!("<show:{ty:?}>");
            CString::new(placeholder)
                .unwrap_or_else(|_| CString::new("?").unwrap())
                .into_raw() as i64
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────

/// Concatena duas C strings, retornando um novo ponteiro Text.
fn text_concat(a: i64, b: i64) -> i64 {
    unsafe { rt::kata_rt_string_concat(a as *const _, b as *const _) as i64 }
}

/// Cria um Text literal a partir de uma string Rust.
fn text_from_str(s: &str) -> Value {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("").unwrap())
        .into_raw() as i64
}

/// show de uma List: `[a b c]` (espaço-separado, colchetes).
fn show_list(val: Value, elem_ty: &Ty, ctx: &InterpCtx) -> Value {
    if val == 0 {
        return text_from_str("[]");
    }

    let mut parts = Vec::new();
    let mut current = val;
    while current != 0 {
        let head = rt::kata_rt_list_head(current);
        let tail = rt::kata_rt_list_tail(current);
        parts.push(show_value(head, elem_ty, ctx));
        current = tail;
    }

    let mut result = text_from_str("[");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result = text_concat(result, text_from_str(" "));
        }
        result = text_concat(result, *part);
    }
    text_concat(result, text_from_str("]"))
}

/// show de um Array: `{a b c}` (espaço-separado, chaves).
fn show_array(val: Value, elem_ty: &Ty, ctx: &InterpCtx) -> Value {
    // kata_rt_array_len retorna SMI-encoded: (len << 1) | 1
    let len = decode_smi(rt::kata_rt_array_len(val));
    if len == 0 {
        return text_from_str("{}");
    }

    let mut parts = Vec::new();
    for i in 0..len {
        let elem = rt::kata_rt_array_get(val, i);
        parts.push(show_value(elem, elem_ty, ctx));
    }

    let mut result = text_from_str("{");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result = text_concat(result, text_from_str(" "));
        }
        result = text_concat(result, *part);
    }
    text_concat(result, text_from_str("}"))
}

/// show de uma Tuple: `(a, b, c)` (vírgula+espaço separado, parênteses).
fn show_tuple(val: Value, elem_tys: &[Ty], ctx: &InterpCtx) -> Value {
    if elem_tys.is_empty() {
        return text_from_str("()");
    }

    let mut parts = Vec::new();
    for (i, elem_ty) in elem_tys.iter().enumerate() {
        let elem = unsafe { std::ptr::read((val as *const Value).add(i)) };
        parts.push(show_value(elem, elem_ty, ctx));
    }

    let mut result = text_from_str("(");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result = text_concat(result, text_from_str(", "));
        }
        result = text_concat(result, *part);
    }
    text_concat(result, text_from_str(")"))
}

/// show de um Struct: `Nome(campo: v, campo: v)`.
fn show_struct(
    val: Value,
    struct_key: &kata_core::struct_registry::StructKey,
    _ty: &Ty,
    ctx: &InterpCtx,
) -> Value {
    // Precisamos dos nomes dos campos e tipos. O StructKey carrega o nome
    // do tipo, mas os campos estão no TypedModule (ctx.module).
    // Para uma implementação correta, precisaríamos acessar o schema do
    // struct. Por enquanto, formatar como Tuple com o nome do tipo.
    let type_name = struct_key.name();
    let n_fields = struct_field_count(struct_key);

    if n_fields == 0 {
        return text_from_str(&format!("{type_name}()"));
    }

    let mut parts = Vec::new();
    for i in 0..n_fields {
        let elem = unsafe { std::ptr::read((val as *const Value).add(i)) };
        // Sem os nomes dos campos nem tipos aqui — usar show genérico
        parts.push(show_value(elem, &Ty::Unit, ctx));
    }

    let mut result = text_from_str(&format!("{type_name}("));
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result = text_concat(result, text_from_str(", "));
        }
        result = text_concat(result, *part);
    }
    text_concat(result, text_from_str(")"))
}

/// show de um Sum/Enum: tag name + payload entre parênteses se houver.
fn show_sum(val: Value, _name: &str, _ty: &Ty, ctx: &InterpCtx) -> Value {
    let tag = rt::kata_rt_sum_tag_int(val);
    // Sem o enum registry para mapear tag → nome, usar o número da tag.
    // Fase 2 pode melhorar isso com acesso ao enum_registry do TypedModule.
    let payload = unsafe { std::ptr::read((val as *const Value).add(1)) };
    if payload == 0 {
        text_from_str(&format!("Variant({tag})"))
    } else {
        let payload_str = show_value(payload, &Ty::Unit, ctx);
        text_concat(
            text_from_str(&format!("Variant({tag}, ")),
            text_concat(payload_str, text_from_str(")")),
        )
    }
}

/// Conta o número de campos de um StructKey.
/// TODO: substituir por acesso ao schema real quando disponível.
fn struct_field_count(_struct_key: &kata_core::struct_registry::StructKey) -> usize {
    // Sem acesso ao schema real aqui. Retornamos 0 por enquanto —
    // structs com campos terão show incompleto até termos o schema.
    0
}
