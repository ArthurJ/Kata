use crate::kata_rt::task;
use crate::kata_rt::memory::SharedMemory;

#[no_mangle]
pub extern "C" fn kata_rt_alloc_local(size: usize, align: usize) -> *mut u8 {
    task::alloc_local(size, align)
}

#[no_mangle]
pub extern "C" fn kata_rt_alloc_shared(size: usize, align: usize) -> *mut u8 {
    SharedMemory::alloc(size, align)
}

#[no_mangle]
pub extern "C" fn kata_rt_decref(ptr: *mut u8) {
    SharedMemory::decref(ptr)
}
