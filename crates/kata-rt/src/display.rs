//! Display de resultados de execução — ponto único de display para
//! driver JIT e shim AOT.
//!
//! `kata_rt_print_result` recebe o valor bruto (`i64`) e um tag de tipo
//! (`i32`) serializado pelo driver. O shim AOT (C) chama esta função
//! diretamente — display vive no runtime, onde já estão BigInt/SMI,
//! Rational, Text, etc.

use std::io::Write;

use crate::bigint::bigint_to_string;
use crate::rational::rat_to_string;

/// Type tag para `kata_rt_print_result` — serializa `ResultTy` do driver.
pub const TYPE_INT: i32 = 0;
pub const TYPE_FLOAT: i32 = 1;
pub const TYPE_TEXT: i32 = 2;
pub const TYPE_RATIONAL: i32 = 3;
pub const TYPE_BOOLEAN: i32 = 4;
pub const TYPE_UNIT: i32 = 5;
pub const TYPE_OTHER: i32 = 6;

/// Imprime o resultado da execução com untagging apropriado.
///
/// `raw` é o valor bruto retornado por `__kata_entry` (i64 para a maioria
/// dos tipos, `f64::to_bits` para Float). `type_tag` é um dos `TYPE_*`.
///
/// # Safety
///
/// `type_tag` deve ser uma constante `TYPE_*` válida. Para `TYPE_TEXT`,
/// `raw` deve ser um ponteiro C string válido. Para `TYPE_RATIONAL`,
/// `raw` deve ser um ponteiro válido para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_print_result(raw: i64, type_tag: i32) {
    match type_tag {
        TYPE_INT => {
            // SMI untag: LSB=1 → SMI (val >> 1); LSB=0 → BigInt pointer.
            let s = bigint_to_string(raw);
            println!("{s}");
        }
        TYPE_FLOAT => {
            // Float: raw é f64 reinterpretado como bits.
            let f = f64::from_bits(raw as u64);
            println!("{f}");
        }
        TYPE_TEXT => {
            // Text: raw é ponteiro para C string.
            // SAFETY: caller garante ponteiro C string válido.
            unsafe {
                let cstr = std::ffi::CStr::from_ptr(raw as *const std::os::raw::c_char);
                println!("{}", cstr.to_string_lossy());
            }
        }
        TYPE_RATIONAL => {
            // Rational: raw é ponteiro para BigRational.
            // SAFETY: caller garante ponteiro válido.
            unsafe {
                let r = &*(raw as *const num_rational::BigRational);
                let s = rat_to_string(r);
                println!("{s}");
            }
        }
        TYPE_BOOLEAN => {
            // Boolean: 1 = True, 0 = False.
            println!("{}", if raw == 1 { "True" } else { "False" });
        }
        TYPE_UNIT => {
            println!("()");
        }
        _ => {
            // Fallback: imprimir valor bruto.
            println!("{raw}");
        }
    }
    let _ = std::io::stdout().flush();
}