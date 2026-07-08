//! I/O — print.

use std::io::Write;

/// Imprime string no stdout. C-ABI: recebe ponteiro C string.
///
/// # Safety
///
/// `s` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_print(s: *const std::os::raw::c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller (JIT codegen) garante ponteiro C string válido ou NULL.
    let cstr = unsafe { std::ffi::CStr::from_ptr(s) };
    let s = cstr.to_string_lossy();
    print!("{s}");
    let _ = std::io::stdout().flush();
}

/// Imprime string com newline.
///
/// # Safety
///
/// `s` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_println(s: *const std::os::raw::c_char) {
    if s.is_null() {
        println!();
        return;
    }
    // SAFETY: caller (JIT codegen) garante ponteiro C string válido ou NULL.
    let cstr = unsafe { std::ffi::CStr::from_ptr(s) };
    let s = cstr.to_string_lossy();
    println!("{s}");
}
