use super::*;
use crate::arena::kata_rt_arena_create;
use crate::bytes::{kata_rt_bytes_get, kata_rt_bytes_len, tag_smi};

struct TestRt {
    rt_ptr: i64,
}
impl TestRt {
    fn new() -> Self {
        let rt = Box::new(crate::runtime::Runtime::new());
        let ptr = Box::into_raw(rt) as i64;
        crate::arena::set_rt_ptr(ptr);
        TestRt { rt_ptr: ptr }
    }
}
impl Drop for TestRt {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.rt_ptr as *mut crate::runtime::Runtime));
        }
    }
}

fn make_arena() -> (TestRt, i64) {
    let rt = TestRt::new();
    let arena = kata_rt_arena_create(rt.rt_ptr);
    (rt, arena)
}

#[test]
fn int_to_bytes() {
    let (_rt, arena) = make_arena();
    let ptr = kata_rt_int_to_bytes(tag_smi(42), arena);
    assert_eq!(untag_smi(kata_rt_bytes_len(ptr)), 4);
    assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 0)), 0x2A); // 42 = 0x2A
    assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 1)), 0x00);
    assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 2)), 0x00);
    assert_eq!(untag_smi(kata_rt_bytes_get(ptr, 3)), 0x00);
}

#[test]
fn text_to_bytes_and_back() {
    let (_rt, arena) = make_arena();
    let text = CString::new("Hello").unwrap();
    let text_ptr = text.as_ptr() as i64;
    let bytes_ptr = unsafe { kata_rt_text_to_bytes(text_ptr, arena) };
    assert_eq!(untag_smi(kata_rt_bytes_len(bytes_ptr)), 5);
    assert_eq!(untag_smi(kata_rt_bytes_get(bytes_ptr, 0)), 0x48);
    // Convert back to text.
    let result_ptr = unsafe { kata_rt_bytes_to_text(bytes_ptr) };
    let s = unsafe { std::ffi::CStr::from_ptr(result_ptr).to_str().unwrap() };
    assert_eq!(s, "Hello");
    unsafe { _ = CString::from_raw(result_ptr) };
}
