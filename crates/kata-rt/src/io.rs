//! I/O — print.

use std::io::Write;

/// Imprime string no stdout. C-ABI: recebe ponteiro C string.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_print(s: *const std::os::raw::c_char) {
    if s.is_null() {
        return;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(s) };
    let s = cstr.to_string_lossy();
    print!("{s}");
    let _ = std::io::stdout().flush();
}

/// Imprime string com newline.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_println(s: *const std::os::raw::c_char) {
    if s.is_null() {
        println!();
        return;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(s) };
    let s = cstr.to_string_lossy();
    println!("{s}");
}
