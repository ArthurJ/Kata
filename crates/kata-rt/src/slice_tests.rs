use super::*;
use crate::arena::kata_rt_arena_create;
use crate::bytes::{tag_smi, untag_smi};

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
fn text_len_codepoints() {
    let text = CString::new("Olá").unwrap(); // O, l, á = 3 codepoints, 4 bytes
    let len = unsafe { kata_rt_text_len(text.as_ptr() as i64) };
    assert_eq!(untag_smi(len), 3); // codepoints, não bytes
}

#[test]
fn text_len_emoji() {
    let text = CString::new("a🚀b").unwrap(); // a, 🚀, b = 3 codepoints, 6 bytes
    let len = unsafe { kata_rt_text_len(text.as_ptr() as i64) };
    assert_eq!(untag_smi(len), 3);
}

#[test]
fn text_at_basic() {
    let (_rt, arena) = make_arena();
    let text = CString::new("ABC").unwrap();
    let result = unsafe { kata_rt_text_at(text.as_ptr() as i64, tag_smi(0), arena) };
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    let payload = unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
    assert_eq!(tag, 0); // Ok
    let s = unsafe {
        std::ffi::CStr::from_ptr(payload as *const std::os::raw::c_char)
            .to_str()
            .unwrap()
    };
    assert_eq!(s, "A");
}

#[test]
fn text_at_unicode() {
    let (_rt, arena) = make_arena();
    let text = CString::new("Olá").unwrap();
    let result = unsafe { kata_rt_text_at(text.as_ptr() as i64, tag_smi(2), arena) };
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    let payload = unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
    assert_eq!(tag, 0); // Ok
    let s = unsafe {
        std::ffi::CStr::from_ptr(payload as *const std::os::raw::c_char)
            .to_str()
            .unwrap()
    };
    assert_eq!(s, "á");
}

#[test]
fn text_at_out_of_bounds() {
    let (_rt, arena) = make_arena();
    let text = CString::new("AB").unwrap();
    let result = unsafe { kata_rt_text_at(text.as_ptr() as i64, tag_smi(5), arena) };
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    assert_eq!(tag, 1); // Err
}

#[test]
fn text_slice_codepoints() {
    let text = CString::new("Hello").unwrap();
    let result = unsafe { kata_rt_text_slice(text.as_ptr() as i64, tag_smi(1), tag_smi(4)) };
    let s = unsafe { std::ffi::CStr::from_ptr(result).to_str().unwrap() };
    assert_eq!(s, "ell");
    unsafe { _ = CString::from_raw(result) };
}

#[test]
fn text_slice_unicode() {
    let text = CString::new("Olá mundo").unwrap();
    let result = unsafe { kata_rt_text_slice(text.as_ptr() as i64, tag_smi(0), tag_smi(3)) };
    let s = unsafe { std::ffi::CStr::from_ptr(result).to_str().unwrap() };
    assert_eq!(s, "Olá");
    unsafe { _ = CString::from_raw(result) };
}

#[test]
fn array_slice() {
    let (_rt, arena) = make_arena();
    let arr = crate::array::kata_rt_array_alloc(5, arena);
    crate::array::kata_rt_array_set(arr, 0, tag_smi(10));
    crate::array::kata_rt_array_set(arr, 1, tag_smi(20));
    crate::array::kata_rt_array_set(arr, 2, tag_smi(30));
    crate::array::kata_rt_array_set(arr, 3, tag_smi(40));
    crate::array::kata_rt_array_set(arr, 4, tag_smi(50));
    let sub = unsafe { kata_rt_array_slice(arr, tag_smi(1), tag_smi(3), arena) };
    assert_eq!(untag_smi(crate::array::kata_rt_array_len(sub)), 2);
    assert_eq!(untag_smi(crate::array::kata_rt_array_get(sub, 0)), 20);
    assert_eq!(untag_smi(crate::array::kata_rt_array_get(sub, 1)), 30);
}

#[test]
fn list_slice() {
    let (_rt, arena) = make_arena();
    // Constrói lista [10, 20, 30, 40, 50]
    let mut list = 0i64;
    for &v in &[50, 40, 30, 20, 10] {
        list = crate::list::kata_rt_list_cons(tag_smi(v), list, arena);
    }
    let sub = unsafe { kata_rt_list_slice(list, tag_smi(1), tag_smi(3), arena) };
    // Deveria ser [20, 30]
    let h1 = unsafe { std::ptr::read_unaligned(sub as *const i64) };
    let t1 = unsafe { std::ptr::read_unaligned((sub as *const u8).add(8) as *const i64) };
    assert_eq!(untag_smi(h1), 20);
    let h2 = unsafe { std::ptr::read_unaligned(t1 as *const i64) };
    assert_eq!(untag_smi(h2), 30);
}
