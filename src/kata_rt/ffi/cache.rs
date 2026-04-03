use std::sync::LazyLock;
use dashmap::DashMap;

static GLOBAL_CACHE: LazyLock<DashMap<i64, usize>> = LazyLock::new(|| DashMap::new());

#[no_mangle]
pub extern "C" fn kata_rt_cache_get(hash: i64) -> *mut u8 {
    if let Some(entry) = GLOBAL_CACHE.get(&hash) {
        *entry.value() as *mut u8
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub extern "C" fn kata_rt_cache_set(hash: i64, val: *mut u8) {
    if !val.is_null() {
        GLOBAL_CACHE.insert(hash, val as usize);
    }
}
