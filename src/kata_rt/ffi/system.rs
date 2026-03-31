use std::ffi::CString;

#[no_mangle]
pub extern "C" fn kata_rt_print_str(ptr: *const u8) {
    if ptr.is_null() {
        println!();
        return;
    }

    // Find length by scanning for null terminator
    let mut len = 0;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
    }

    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    if let Ok(s) = std::str::from_utf8(slice) {
        println!("{}", s);
    }
}

#[no_mangle]
pub extern "C" fn kata_rt_panic() {
    std::process::exit(1);
}

#[no_mangle]
pub extern "C" fn kata_rt_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

/// Converte um inteiro para string (retorna ponteiro para string alocada)
/// O chamador é responsável por liberar a memória com kata_rt_str_free
#[no_mangle]
pub extern "C" fn kata_rt_int_to_str(value: i64) -> *mut u8 {
    let s = value.to_string();
    let c_string = CString::new(s).unwrap();
    let bytes = c_string.into_bytes_with_nul();
    let boxed = bytes.into_boxed_slice();
    Box::into_raw(boxed) as *mut u8
}

/// Converte um float para string (retorna ponteiro para string alocada)
#[no_mangle]
pub extern "C" fn kata_rt_flt_to_str(value: f64) -> *mut u8 {
    let s = value.to_string();
    let c_string = CString::new(s).unwrap();
    let bytes = c_string.into_bytes_with_nul();
    let boxed = bytes.into_boxed_slice();
    Box::into_raw(boxed) as *mut u8
}

/// Converte um booleano para string
#[no_mangle]
pub extern "C" fn kata_rt_bool_to_str(value: bool) -> *mut u8 {
    let s = if value { "true" } else { "false" };
    let c_string = CString::new(s).unwrap();
    let bytes = c_string.into_bytes_with_nul();
    let boxed = bytes.into_boxed_slice();
    Box::into_raw(boxed) as *mut u8
}

/// Concatena duas strings (strings terminadas em null)
/// Retorna uma nova string alocada
#[no_mangle]
pub extern "C" fn kata_rt_concat_text(ptr1: *const u8, len1: usize, ptr2: *const u8, len2: usize) -> *mut u8 {
    let slice1 = unsafe { std::slice::from_raw_parts(ptr1, len1) };
    let slice2 = unsafe { std::slice::from_raw_parts(ptr2, len2) };

    let mut result = Vec::with_capacity(len1 + len2 + 1);
    result.extend_from_slice(slice1);
    result.extend_from_slice(slice2);
    result.push(0); // null terminator

    let boxed = result.into_boxed_slice();
    Box::into_raw(boxed) as *mut u8
}

/// Libera memória de string alocada
#[no_mangle]
pub extern "C" fn kata_rt_str_free(ptr: *mut u8) {
    if !ptr.is_null() {
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }
}

/// Retorna o tamanho de uma string (sem o null terminator)
#[no_mangle]
pub extern "C" fn kata_rt_str_len(ptr: *const u8) -> usize {
    if ptr.is_null() {
        return 0;
    }
    unsafe {
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        len
    }
}

/// Representação padrão para tipos que implementam REPR
/// Retorna um ponteiro para string alocada (o chamador deve liberar com kata_rt_str_free)
#[no_mangle]
pub extern "C" fn kata_rt_default_repr(ptr: *const u8, len: usize) -> *mut u8 {
    // Por enquanto retorna uma representação simples
    // TODO: Implementar representação real baseada no tipo
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let s = format!("{:?}", slice);
    let c_string = CString::new(s).unwrap();
    let bytes = c_string.into_bytes_with_nul();
    let boxed = bytes.into_boxed_slice();
    Box::into_raw(boxed) as *mut u8
}

/// Igualdade genérica para tipos
/// Compara ponteiros diretamente (implementação simplificada)
#[no_mangle]
pub extern "C" fn kata_rt_eq_generic(a: *const u8, b: *const u8) -> bool {
    a == b
}
