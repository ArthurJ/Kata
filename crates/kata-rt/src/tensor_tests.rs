//! Testes do runtime de tensor — Fase 1 do PRD-tensor.
//!
//! Convenção: dados Int são SMI-tagged (tag_smi), dados Float são bits de f64.
//! Isso espelha a ABI do runtime Kata — Int é sempre SMI, Float é sempre f64::to_bits.

use crate::bytes::{tag_smi, untag_smi};
use crate::tensor::*;

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

/// Cria um tensor Int 2×3 com dados SMI-tagged [1,2,3,4,5,6] (row-major).
fn make_2x3_int() -> (TestRt, i64) {
    let rt = TestRt::new();
    let data: [i64; 6] = [
        tag_smi(1),
        tag_smi(2),
        tag_smi(3),
        tag_smi(4),
        tag_smi(5),
        tag_smi(6),
    ];
    let shape: [i64; 2] = [2, 3];
    let ptr = kata_rt_tensor_new(data.as_ptr() as i64, 2, shape.as_ptr() as i64, ELEM_INT);
    (rt, ptr)
}

/// Cria um tensor Int 2×2 com dados SMI-tagged.
fn make_2x2_int(vals: [i64; 4]) -> (TestRt, i64) {
    let rt = TestRt::new();
    let data: [i64; 4] = [
        tag_smi(vals[0]),
        tag_smi(vals[1]),
        tag_smi(vals[2]),
        tag_smi(vals[3]),
    ];
    let shape: [i64; 2] = [2, 2];
    let ptr = kata_rt_tensor_new(data.as_ptr() as i64, 2, shape.as_ptr() as i64, ELEM_INT);
    (rt, ptr)
}

/// Cria um tensor Float 2×2 com dados [1.0, 2.0, 3.0, 4.0].
fn make_2x2_float(vals: [f64; 4]) -> (TestRt, i64) {
    let rt = TestRt::new();
    let data: [u64; 4] = [
        vals[0].to_bits(),
        vals[1].to_bits(),
        vals[2].to_bits(),
        vals[3].to_bits(),
    ];
    let shape: [i64; 2] = [2, 2];
    let ptr = kata_rt_tensor_new(data.as_ptr() as i64, 2, shape.as_ptr() as i64, ELEM_FLOAT);
    (rt, ptr)
}

/// Lê o payload de um Result box (tag + payload).
fn read_result(result: i64) -> (i64, i64) {
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    let payload = unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
    (tag, payload)
}

/// Lê o valor Int de um tensor no índice flatten (já desempacota SMI).
fn read_int(tensor: i64, flat: i64) -> i64 {
    let r = kata_rt_tensor_at(tensor, tag_smi(flat));
    let (tag, payload) = read_result(r);
    assert_eq!(tag, 0, "esperado Ok no índice {flat}");
    untag_smi(payload)
}

/// Lê o valor Float de um tensor no índice flatten.
fn read_float(tensor: i64, flat: i64) -> f64 {
    let r = kata_rt_tensor_at(tensor, tag_smi(flat));
    let (tag, payload) = read_result(r);
    assert_eq!(tag, 0, "esperado Ok no índice {flat}");
    f64::from_bits(payload as u64)
}

// ── Tensor new + rank + shape ──────────────────────────────────────

#[test]
fn tensor_new_basic() {
    let (_rt, ptr) = make_2x3_int();
    assert!(ptr != 0, "tensor não deve ser nulo");
    let rank = untag_smi(kata_rt_tensor_rank(ptr));
    assert_eq!(rank, 2);
}

#[test]
fn tensor_shape_basic() {
    let (_rt, ptr) = make_2x3_int();
    let shape_ptr = kata_rt_tensor_shape(ptr) as *const i64;
    assert!(!shape_ptr.is_null());
    let s0 = unsafe { std::ptr::read_unaligned(shape_ptr) };
    let s1 = unsafe { std::ptr::read_unaligned(shape_ptr.add(1)) };
    assert_eq!(s0, 2);
    assert_eq!(s1, 3);
}

// ── at (indexação flatten) ─────────────────────────────────────────

#[test]
fn tensor_at_basic() {
    let (_rt, ptr) = make_2x3_int();
    assert_eq!(read_int(ptr, 0), 1);
    assert_eq!(read_int(ptr, 1), 2);
    assert_eq!(read_int(ptr, 5), 6);
}

#[test]
fn tensor_at_out_of_bounds() {
    let (_rt, ptr) = make_2x3_int();
    let result = kata_rt_tensor_at(ptr, tag_smi(6));
    let (tag, _payload) = read_result(result);
    assert_eq!(tag, 1, "deve ser Err");
}

#[test]
fn tensor_at_negative() {
    let (_rt, ptr) = make_2x3_int();
    let result = kata_rt_tensor_at(ptr, tag_smi(-1));
    let (tag, _payload) = read_result(result);
    assert_eq!(tag, 1, "deve ser Err");
}

// ── add (element-wise + broadcast) ─────────────────────────────────

#[test]
fn tensor_add_2x2() {
    let (_rt, a) = make_2x2_int([1, 2, 3, 4]);
    let (_rt2, b) = make_2x2_int([5, 6, 7, 8]);

    let result = kata_rt_tensor_add(a, b);
    let (tag, tensor_ptr) = read_result(result);
    assert_eq!(tag, 0, "add deve ser Ok");

    // [1+5, 2+6, 3+7, 4+8] = [6, 8, 10, 12]
    assert_eq!(read_int(tensor_ptr, 0), 6);
    assert_eq!(read_int(tensor_ptr, 1), 8);
    assert_eq!(read_int(tensor_ptr, 2), 10);
    assert_eq!(read_int(tensor_ptr, 3), 12);
}

#[test]
fn tensor_add_broadcast() {
    let _rt = TestRt::new();
    // a = [1 2 3; 4 5 6] (2×3)
    let a_data: [i64; 6] = [
        tag_smi(1),
        tag_smi(2),
        tag_smi(3),
        tag_smi(4),
        tag_smi(5),
        tag_smi(6),
    ];
    let a_shape: [i64; 2] = [2, 3];
    let a = kata_rt_tensor_new(a_data.as_ptr() as i64, 2, a_shape.as_ptr() as i64, ELEM_INT);
    // b = [10 20 30] (1×3) — broadcast na dim 0
    let b_data: [i64; 3] = [tag_smi(10), tag_smi(20), tag_smi(30)];
    let b_shape: [i64; 2] = [1, 3];
    let b = kata_rt_tensor_new(b_data.as_ptr() as i64, 2, b_shape.as_ptr() as i64, ELEM_INT);

    let result = kata_rt_tensor_add(a, b);
    let (tag, tensor_ptr) = read_result(result);
    assert_eq!(tag, 0, "broadcast add deve ser Ok");

    // Esperado: [11 22 33; 14 25 36]
    let expected = [11, 22, 33, 14, 25, 36];
    for i in 0..6 {
        assert_eq!(read_int(tensor_ptr, i), expected[i as usize]);
    }
}

#[test]
fn tensor_add_incompatible_shapes() {
    let (_rt, a) = make_2x2_int([1, 2, 3, 4]);

    let _rt2 = TestRt::new();
    let b_data: [i64; 3] = [tag_smi(1), tag_smi(2), tag_smi(3)];
    let b_shape: [i64; 2] = [1, 3]; // 1×3 não broadcastable com 2×2
    let b = kata_rt_tensor_new(b_data.as_ptr() as i64, 2, b_shape.as_ptr() as i64, ELEM_INT);

    let result = kata_rt_tensor_add(a, b);
    let (tag, _payload) = read_result(result);
    assert_eq!(tag, 1, "shapes incompatíveis devem ser Err");
}

// ── mul (Hadamard) ─────────────────────────────────────────────────

#[test]
fn tensor_mul_2x2() {
    let (_rt, a) = make_2x2_int([1, 2, 3, 4]);
    let (_rt2, b) = make_2x2_int([5, 6, 7, 8]);

    let result = kata_rt_tensor_mul(a, b);
    let (tag, tensor_ptr) = read_result(result);
    assert_eq!(tag, 0);

    // [1*5, 2*6, 3*7, 4*8] = [5, 12, 21, 32]
    assert_eq!(read_int(tensor_ptr, 0), 5);
    assert_eq!(read_int(tensor_ptr, 1), 12);
    assert_eq!(read_int(tensor_ptr, 2), 21);
    assert_eq!(read_int(tensor_ptr, 3), 32);
}

// ── panic_add / panic_mul ──────────────────────────────────────────

#[test]
fn tensor_panic_add_ok() {
    let (_rt, a) = make_2x2_int([1, 2, 3, 4]);
    let (_rt2, b) = make_2x2_int([5, 6, 7, 8]);

    let tensor_ptr = kata_rt_tensor_panic_add(a, b);
    assert!(tensor_ptr != 0);
    assert_eq!(read_int(tensor_ptr, 0), 6);
}

// ── dot ────────────────────────────────────────────────────────────

#[test]
fn tensor_dot_2x2_int() {
    // a = [1 2; 3 4], b = [5 6; 7 8]
    // dot = [1*5+2*7, 1*6+2*8; 3*5+4*7, 3*6+4*8] = [19 22; 43 50]
    let (_rt, a) = make_2x2_int([1, 2, 3, 4]);
    let (_rt2, b) = make_2x2_int([5, 6, 7, 8]);

    let result = kata_rt_tensor_dot(a, b);
    let (tag, tensor_ptr) = read_result(result);
    assert_eq!(tag, 0);

    assert_eq!(read_int(tensor_ptr, 0), 19);
    assert_eq!(read_int(tensor_ptr, 1), 22);
    assert_eq!(read_int(tensor_ptr, 2), 43);
    assert_eq!(read_int(tensor_ptr, 3), 50);
}

#[test]
fn tensor_dot_1d_1d() {
    // [1 2 3] · [4 5 6] = 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
    let _rt = TestRt::new();
    let a_data: [i64; 3] = [tag_smi(1), tag_smi(2), tag_smi(3)];
    let b_data: [i64; 3] = [tag_smi(4), tag_smi(5), tag_smi(6)];
    let shape: [i64; 1] = [3];
    let a = kata_rt_tensor_new(a_data.as_ptr() as i64, 1, shape.as_ptr() as i64, ELEM_INT);
    let b = kata_rt_tensor_new(b_data.as_ptr() as i64, 1, shape.as_ptr() as i64, ELEM_INT);

    let result = kata_rt_tensor_dot(a, b);
    let (tag, tensor_ptr) = read_result(result);
    assert_eq!(tag, 0);

    assert_eq!(read_int(tensor_ptr, 0), 32);
}

#[test]
fn tensor_dot_2d_1d() {
    // a = [1 2; 3 4] (2×2), b = [5 6] (2,)
    // dot = [1*5+2*6, 3*5+4*6] = [17, 39]
    let (_rt, a) = make_2x2_int([1, 2, 3, 4]);

    let _rt2 = TestRt::new();
    let b_data: [i64; 2] = [tag_smi(5), tag_smi(6)];
    let b_shape: [i64; 1] = [2];
    let b = kata_rt_tensor_new(b_data.as_ptr() as i64, 1, b_shape.as_ptr() as i64, ELEM_INT);

    let result = kata_rt_tensor_dot(a, b);
    let (tag, tensor_ptr) = read_result(result);
    assert_eq!(tag, 0);

    assert_eq!(read_int(tensor_ptr, 0), 17);
    assert_eq!(read_int(tensor_ptr, 1), 39);
}

#[test]
fn tensor_dot_float_2x2() {
    // a = [1.0 2.0; 3.0 4.0], b = [1.0 2.0; 3.0 4.0]
    // dot = [1*1+2*3, 1*2+2*4; 3*1+4*3, 3*2+4*4] = [7 10; 15 22]
    let (_rt, a) = make_2x2_float([1.0, 2.0, 3.0, 4.0]);
    let (_rt2, b) = make_2x2_float([1.0, 2.0, 3.0, 4.0]);

    let result = kata_rt_tensor_dot(a, b);
    let (tag, tensor_ptr) = read_result(result);
    assert_eq!(tag, 0);

    let expected = [7.0, 10.0, 15.0, 22.0];
    for (i, &exp) in expected.iter().enumerate() {
        let val = read_float(tensor_ptr, i as i64);
        assert!(
            (val - exp).abs() < 1e-10,
            "dot[{i}] = {val}, esperado {exp}"
        );
    }
}

#[test]
fn tensor_dot_inner_dims_mismatch() {
    let _rt = TestRt::new();
    let a_data: [i64; 6] = [
        tag_smi(1),
        tag_smi(2),
        tag_smi(3),
        tag_smi(4),
        tag_smi(5),
        tag_smi(6),
    ];
    let a_shape: [i64; 2] = [2, 3]; // inner dim = 3
    let a = kata_rt_tensor_new(a_data.as_ptr() as i64, 2, a_shape.as_ptr() as i64, ELEM_INT);

    let b_data: [i64; 4] = [tag_smi(1), tag_smi(2), tag_smi(3), tag_smi(4)];
    let b_shape: [i64; 2] = [2, 2]; // inner dim = 2
    let b = kata_rt_tensor_new(b_data.as_ptr() as i64, 2, b_shape.as_ptr() as i64, ELEM_INT);

    let result = kata_rt_tensor_dot(a, b);
    let (tag, _payload) = read_result(result);
    assert_eq!(tag, 1, "inner dims não casam deve ser Err");
}

// ── transpose ─────────────────────────────────────────────────────

#[test]
fn tensor_transpose_2x3() {
    let (_rt, ptr) = make_2x3_int();
    // [1 2 3; 4 5 6] → [1 4; 2 5; 3 6] (zero-copy: muda strides, não rearranja data)
    let result = kata_rt_tensor_transpose(ptr);
    assert!(result != 0);

    // Shape deve ser 3×2
    let shape_ptr = kata_rt_tensor_shape(result) as *const i64;
    assert_eq!(unsafe { std::ptr::read_unaligned(shape_ptr) }, 3);
    assert_eq!(unsafe { std::ptr::read_unaligned(shape_ptr.add(1)) }, 2);

    // Strides devem ser transpostos: original [3,1] → [1,3]
    // No tensor transposto, acessar (i,j) deve dar o elemento (j,i) do original
    // Flatten no transposto: flat = i*stride[0] + j*stride[1]
    // Como stride = [1,3], flat(i,j) = i*1 + j*3 = i + 3*j
    // (0,0)→flat 0→elem 0 do data original = 1 ✓
    // (0,1)→flat 3→elem 3 do data original = 4 ✓ (era (1,0) do original)
    // (1,0)→flat 1→elem 1 do data original = 2 ✓ (era (0,1) do original)
    // (2,1)→flat 5→elem 5 do data original = 6 ✓
    let expected = [1, 2, 3, 4, 5, 6]; // ordem flatten do transposto com strides [1,3]
    for i in 0..6 {
        assert_eq!(read_int(result, i), expected[i as usize]);
    }
}

// ── scale / shift ──────────────────────────────────────────────────

#[test]
fn tensor_scale_int() {
    let (_rt, ptr) = make_2x3_int();
    // [1 2 3; 4 5 6] * 2 = [2 4 6; 8 10 12]
    let result = kata_rt_tensor_scale(ptr, tag_smi(2));
    assert!(result != 0);

    let expected = [2, 4, 6, 8, 10, 12];
    for i in 0..6 {
        assert_eq!(read_int(result, i), expected[i as usize]);
    }
}

#[test]
fn tensor_shift_int() {
    let (_rt, ptr) = make_2x3_int();
    // [1 2 3; 4 5 6] + 10 = [11 12 13; 14 15 16]
    let result = kata_rt_tensor_shift(ptr, tag_smi(10));
    assert!(result != 0);

    let expected = [11, 12, 13, 14, 15, 16];
    for i in 0..6 {
        assert_eq!(read_int(result, i), expected[i as usize]);
    }
}

// ── show (display) ─────────────────────────────────────────────────

#[test]
fn tensor_show_2d() {
    let _rt = TestRt::new();
    let data: [i64; 9] = [
        tag_smi(1),
        tag_smi(22),
        tag_smi(333),
        tag_smi(4444),
        tag_smi(55),
        tag_smi(6),
        tag_smi(7),
        tag_smi(88888),
        tag_smi(99),
    ];
    let shape: [i64; 2] = [3, 3];
    let ptr = kata_rt_tensor_new(data.as_ptr() as i64, 2, shape.as_ptr() as i64, ELEM_INT);

    let str_ptr = kata_rt_tensor_show(ptr);
    let s = unsafe {
        std::ffi::CStr::from_ptr(str_ptr as *const std::os::raw::c_char)
            .to_string_lossy()
            .into_owned()
    };
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 3, "deve ter 3 linhas para 3×3");
    // Primeira linha contém "1", "22", "333" com padding
    assert!(lines[0].trim_start().starts_with("1"));
    assert!(lines[0].contains("22"));
    assert!(lines[0].trim_end().ends_with("333"));
}

#[test]
fn tensor_show_1d() {
    let _rt = TestRt::new();
    let data: [i64; 3] = [tag_smi(1), tag_smi(2), tag_smi(3)];
    let shape: [i64; 1] = [3];
    let ptr = kata_rt_tensor_new(data.as_ptr() as i64, 1, shape.as_ptr() as i64, ELEM_INT);

    let str_ptr = kata_rt_tensor_show(ptr);
    let s = unsafe {
        std::ffi::CStr::from_ptr(str_ptr as *const std::os::raw::c_char)
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(s, "1  2  3");
}

// ── Float operations ──────────────────────────────────────────────

#[test]
fn tensor_add_float() {
    let (_rt, a) = make_2x2_float([1.0, 2.0, 3.0, 4.0]);
    let (_rt2, b) = make_2x2_float([10.0, 20.0, 30.0, 40.0]);

    let result = kata_rt_tensor_add(a, b);
    let (tag, tensor_ptr) = read_result(result);
    assert_eq!(tag, 0);

    let expected = [11.0, 22.0, 33.0, 44.0];
    for (i, &exp) in expected.iter().enumerate() {
        let val = read_float(tensor_ptr, i as i64);
        assert!((val - exp).abs() < 1e-10);
    }
}
