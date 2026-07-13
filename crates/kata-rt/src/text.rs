//! Text (strings UTF-8).
//!
//! Strings são "dados cegas" — o lexer não interpreta conteúdo.
//! Sem interpolação léxica embutida (injeção delegada a `format`).

use std::ffi::CString;

/// Concatena duas strings.
///
/// # Safety
///
/// `a` e `b` devem ser ponteiros C string válidos (nulo-terminados).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_string_concat(
    a: *const std::os::raw::c_char,
    b: *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char {
    // SAFETY: caller (JIT codegen) garante ponteiros C string válidos.
    let a = unsafe { std::ffi::CStr::from_ptr(a).to_string_lossy().into_owned() };
    let b = unsafe { std::ffi::CStr::from_ptr(b).to_string_lossy().into_owned() };
    let result = format!("{a}{b}");
    CString::new(result)
        .unwrap_or_else(|_| CString::new("").expect("CString vazia sempre válida"))
        .into_raw()
}

/// Comprimento de string em bytes.
///
/// # Safety
///
/// `s` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_string_len(s: *const std::os::raw::c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    // SAFETY: caller (JIT codegen) garante ponteiro C string válido ou NULL.
    unsafe { std::ffi::CStr::from_ptr(s).to_bytes().len() as i64 }
}

/// Cria string literal a partir de texto (para codegen de TextLit).
pub fn text_literal(s: &str) -> CString {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("").expect("empty string never contains nul bytes"))
}

/// Converte Int (i64 tagged) para String.
pub fn int_to_text(val: i64) -> String {
    crate::bigint::bigint_to_string(val)
}

/// Converte Boolean (0/1) para String "True"/"False".
pub fn bool_to_text(val: i64) -> String {
    if val == 1 {
        "True".into()
    } else {
        "False".into()
    }
}

/// Substitui primeira ocorrência de `{}` por valor (para `format`).
pub fn text_replace_first(template: &str, replacement: &str) -> String {
    if let Some(pos) = template.find("{}") {
        format!(
            "{}{}{}",
            &template[..pos],
            replacement,
            &template[pos + 2..]
        )
    } else {
        template.to_string()
    }
}

// ── Wrappers C-ABI para codegen ───────────────────────────

/// Cria string literal a partir de texto (ponteiro + len).
/// Retorna ponteiro C string (ownership transferida ao caller).
/// Chamado pelo codegen via `FfiSymbol::TextLiteral`.
///
/// # Safety
///
/// `s` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
/// Se não for NULL, `len` deve ser o comprimento em bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_text_literal(
    s: *const std::os::raw::c_char,
    len: i64,
) -> *mut std::os::raw::c_char {
    let bytes = if s.is_null() || len <= 0 {
        &b""[..]
    } else {
        // SAFETY: caller (JIT codegen) garante ponteiro e len válidos.
        unsafe { std::slice::from_raw_parts(s as *const u8, len as usize) }
    };
    let text = std::str::from_utf8(bytes).unwrap_or("");
    std::ffi::CString::new(text)
        .unwrap_or_else(|_| {
            std::ffi::CString::new("").expect("empty string never contains nul bytes")
        })
        .into_raw()
}

/// Converte Boolean (0/1) para Text "True"/"False" (ponteiro C string).
/// Chamado pelo codegen via `FfiSymbol::BoolToText`.
///
/// # Safety
///
/// Esta função é safe para qualquer `i64` — não dereferencia ponteiros.
/// Marcada `unsafe` apenas por convenção C-ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_bool_to_text(val: i64) -> *mut std::os::raw::c_char {
    let s = bool_to_text(val);
    std::ffi::CString::new(s)
        .unwrap_or_else(|_| {
            std::ffi::CString::new("").expect("empty string never contains nul bytes")
        })
        .into_raw()
}

/// Substitui primeira ocorrência de `{}` por valor (ponteiro C string).
/// Chamado pelo codegen via `FfiSymbol::TextReplaceFirst`.
///
/// # Safety
///
/// `template` e `replacement` devem ser ponteiros C string válidos ou NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_text_replace_first(
    template: *const std::os::raw::c_char,
    replacement: *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char {
    let template = if template.is_null() {
        String::new()
    } else {
        // SAFETY: caller (JIT codegen) garante ponteiro C string válido ou NULL.
        unsafe {
            std::ffi::CStr::from_ptr(template)
                .to_string_lossy()
                .into_owned()
        }
    };
    let replacement = if replacement.is_null() {
        String::new()
    } else {
        // SAFETY: caller (JIT codegen) garante ponteiro C string válido ou NULL.
        unsafe {
            std::ffi::CStr::from_ptr(replacement)
                .to_string_lossy()
                .into_owned()
        }
    };
    let result = text_replace_first(&template, &replacement);
    std::ffi::CString::new(result)
        .unwrap_or_else(|_| {
            std::ffi::CString::new("").expect("empty string never contains nul bytes")
        })
        .into_raw()
}
