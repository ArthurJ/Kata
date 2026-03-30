#[no_mangle]
pub extern "C" fn kata_rt_print_str(ptr: *const u8, len: usize) {
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
