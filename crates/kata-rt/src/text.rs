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
    let len = unsafe { std::ffi::CStr::from_ptr(s).to_bytes().len() as i64 };
    // SMI tag: (val << 1) | 1 — consistente com kata_rt_list_len e todo Int.
    (len << 1) | 1
}

/// Converte Boolean (0/1) para String "True"/"False".
pub(crate) fn bool_to_text(val: i64) -> String {
    if val == 1 {
        "True".into()
    } else {
        "False".into()
    }
}

/// Compara duas strings C por conteúdo. Retorna 1 se iguais, 0 se diferentes.
///
/// Usado como função de igualdade (eq_fn) para chaves Text em Dict/Set.
///
/// # Safety
///
/// `a` e `b` são ponteiros i64 para C strings (nulo-terminadas) ou 0 (NULL).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_string_eq(a: i64, b: i64) -> i64 {
    if a == 0 && b == 0 {
        return 1;
    }
    if a == 0 || b == 0 {
        return 0;
    }
    // SAFETY: caller (JIT codegen) garante ponteiros C string válidos.
    let a_bytes = unsafe { std::ffi::CStr::from_ptr(a as *const std::os::raw::c_char).to_bytes() };
    let b_bytes = unsafe { std::ffi::CStr::from_ptr(b as *const std::os::raw::c_char).to_bytes() };
    if a_bytes == b_bytes { 1 } else { 0 }
}

/// Verifica se `haystack` começa com `needle`. Retorna 1 se verdadeiro, 0 se falso.
///
/// # Safety
///
/// `haystack` e `needle` são ponteiros i64 para C strings (nulo-terminadas) ou 0 (NULL).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_string_starts_with(haystack: i64, needle: i64) -> i64 {
    if needle == 0 {
        return 1; // prefixo vazio sempre casa
    }
    if haystack == 0 {
        return 0;
    }
    // SAFETY: caller (JIT codegen) garante ponteiros C string válidos.
    let h = unsafe { std::ffi::CStr::from_ptr(haystack as *const std::os::raw::c_char).to_bytes() };
    let n = unsafe { std::ffi::CStr::from_ptr(needle as *const std::os::raw::c_char).to_bytes() };
    if h.starts_with(n) { 1 } else { 0 }
}

/// Verifica se `haystack` contém `needle` como substring. Retorna 1 se verdadeiro, 0 se falso.
///
/// # Safety
///
/// `haystack` e `needle` são ponteiros i64 para C strings (nulo-terminadas) ou 0 (NULL).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_string_contains(haystack: i64, needle: i64) -> i64 {
    if needle == 0 {
        return 1; // substring vazia sempre está contida
    }
    if haystack == 0 {
        return 0;
    }
    // SAFETY: caller (JIT codegen) garante ponteiros C string válidos.
    let h = unsafe { std::ffi::CStr::from_ptr(haystack as *const std::os::raw::c_char).to_bytes() };
    let n = unsafe { std::ffi::CStr::from_ptr(needle as *const std::os::raw::c_char).to_bytes() };
    if n.is_empty() {
        1
    } else if h.windows(n.len()).any(|w| w == n) {
        1
    } else {
        0
    }
}

/// Substitui primeira ocorrência de `{}` por valor (para `format`).
pub(crate) fn text_replace_first(template: &str, replacement: &str) -> String {
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

/// Substitui primeira ocorrência de `needle` por `replacement` (3 args).
/// Usado por `format!` nomeado: substitui `{key}` por valor.
///
/// # Safety
///
/// `template`, `needle`, `replacement` devem ser ponteiros C string válidos ou NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_text_replace(
    template: *const std::os::raw::c_char,
    needle: *const std::os::raw::c_char,
    replacement: *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char {
    let template = if template.is_null() {
        String::new()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(template)
                .to_string_lossy()
                .into_owned()
        }
    };
    let needle = if needle.is_null() {
        String::new()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(needle)
                .to_string_lossy()
                .into_owned()
        }
    };
    let replacement = if replacement.is_null() {
        String::new()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(replacement)
                .to_string_lossy()
                .into_owned()
        }
    };
    let result = if let Some(pos) = template.find(&needle) {
        format!(
            "{}{}{}",
            &template[..pos],
            replacement,
            &template[pos + needle.len()..]
        )
    } else {
        template
    };
    std::ffi::CString::new(result)
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
