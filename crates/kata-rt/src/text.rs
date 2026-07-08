//! Text (strings UTF-8).
//!
//! Strings são "dados cegos" — o lexer não interpreta conteúdo.
//! Sem interpolação léxica embutida (injeção delegada a `format`).

use std::ffi::CString;

/// Concatena duas strings.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_string_concat(
    a: *const std::os::raw::c_char,
    b: *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char {
    let a = unsafe { std::ffi::CStr::from_ptr(a).to_string_lossy().into_owned() };
    let b = unsafe { std::ffi::CStr::from_ptr(b).to_string_lossy().into_owned() };
    let result = format!("{a}{b}");
    CString::new(result)
        .unwrap_or_else(|_| CString::new("").unwrap())
        .into_raw()
}

/// Comprimento de string em bytes.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_string_len(s: *const std::os::raw::c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    unsafe { std::ffi::CStr::from_ptr(s).to_bytes().len() as i64 }
}

/// Cria string literal a partir de texto (para codegen de TextLit).
pub fn text_literal(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
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
