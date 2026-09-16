//! Assinaturas FFI para operações de Tensor.

use crate::call_conv::ffi_call_conv;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de tensor.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(ffi_call_conv());
    match sym {
        // tensor_new: (data, rank, shape, elem_type) -> i64 (tensor ptr)
        FfiSymbol::TensorNew => {
            sig.params.push(AbiParam::new(I64)); // data
            sig.params.push(AbiParam::new(I64)); // rank
            sig.params.push(AbiParam::new(I64)); // shape
            sig.params.push(AbiParam::new(I64)); // elem_type
            sig.returns.push(AbiParam::new(I64)); // tensor ptr
        }
        // tensor_rank: (ptr) -> i64
        FfiSymbol::TensorRank => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // rank
        }
        // tensor_shape: (ptr) -> i64
        FfiSymbol::TensorShape => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // shape ptr
        }
        // tensor_at: (ptr, idx) -> i64
        FfiSymbol::TensorAt => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.returns.push(AbiParam::new(I64)); // value
        }
        // tensor_at_nd: (ptr, indices_ptr, n) -> i64 (Result box)
        FfiSymbol::TensorAtNd => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // indices_ptr
            sig.params.push(AbiParam::new(I64)); // n_indices
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // tensor_sub: (ptr, starts_ptr, ends_ptr, n, mask) -> i64 (tensor ptr)
        FfiSymbol::TensorSub => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // starts_ptr
            sig.params.push(AbiParam::new(I64)); // ends_ptr
            sig.params.push(AbiParam::new(I64)); // n_axes
            sig.params.push(AbiParam::new(I64)); // collapse_mask
            sig.returns.push(AbiParam::new(I64)); // tensor ptr
        }
        // tensor_add: (a, b) -> i64 (Result box)
        FfiSymbol::TensorAdd => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // tensor_mul: (a, b) -> i64 (Result box)
        FfiSymbol::TensorMul => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // tensor_panic_add: (a, b) -> i64
        FfiSymbol::TensorPanicAdd => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // tensor ptr
        }
        // tensor_panic_mul: (a, b) -> i64
        FfiSymbol::TensorPanicMul => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // tensor ptr
        }
        // tensor_dot: (a, b) -> i64 (Result box)
        FfiSymbol::TensorDot => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // tensor_transpose: (ptr) -> i64 (Result box)
        FfiSymbol::TensorTranspose => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // tensor_scale: (ptr, scalar) -> i64 (Result box)
        FfiSymbol::TensorScale => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // scalar
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // tensor_shift: (ptr, scalar) -> i64 (Result box)
        FfiSymbol::TensorShift => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // scalar
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // tensor_free: (ptr) -> ()
        FfiSymbol::TensorFree => {
            sig.params.push(AbiParam::new(I64)); // ptr
        }
        // tensor_show: (ptr) -> i64 (Text ptr)
        FfiSymbol::TensorShow => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // Text ptr
        }
        // tensor_eye_int: (n) -> i64 (tensor ptr)
        FfiSymbol::TensorEyeInt => {
            sig.params.push(AbiParam::new(I64)); // n (SMI-tagged)
            sig.returns.push(AbiParam::new(I64)); // tensor ptr
        }
        // tensor_eye_float: (n) -> i64 (tensor ptr)
        FfiSymbol::TensorEyeFloat => {
            sig.params.push(AbiParam::new(I64)); // n (SMI-tagged)
            sig.returns.push(AbiParam::new(I64)); // tensor ptr
        }
        // tensor_zeros_int: (shape_ptr) -> i64 (tensor ptr)
        FfiSymbol::TensorZerosInt => {
            sig.params.push(AbiParam::new(I64)); // shape_ptr (Array)
            sig.returns.push(AbiParam::new(I64)); // tensor ptr
        }
        // tensor_zeros_float: (shape_ptr) -> i64 (tensor ptr)
        FfiSymbol::TensorZerosFloat => {
            sig.params.push(AbiParam::new(I64)); // shape_ptr (Array)
            sig.returns.push(AbiParam::new(I64)); // tensor ptr
        }
        _ => return None,
    }
    Some(sig)
}
