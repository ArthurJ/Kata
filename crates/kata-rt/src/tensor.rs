//! Tensor — buffer N-D contíguo com álgebra linear.
//!
//! Layout de memória (single arena allocation):
//! ```text
//! struct TensorHeader {
//!     data:     *mut u8,   // buffer contíguo, row-major
//!     rank:     i64,        // ≥ 1 — tensores 0-D não existem
//!     shape:    *mut i64,   // [rank] dimensões
//!     strides:  *mut i64,   // [rank] strides em elementos (não bytes)
//!     elem_type: i64,      // 0 = Int, 1 = Float (PrimTy tag)
//!     elem_size: i64,      // 8 para Int, 8 para Float (bytes por elemento)
//! }
//! ```
//!
//! O header e todos os arrays (shape, strides, data) são alocados em um
//! único bloco contíguo na arena. `kata_rt_tensor_free` libera tudo.
//! `transpose` é zero-copy: troca strides no header, reusa o data buffer.

use std::ffi::CString;

use crate::bytes::{tag_smi, untag_smi};

/// Tag de tipo de elemento (espelha PrimTy do compilador).
pub(crate) const ELEM_INT: i64 = 0;
pub(crate) const ELEM_FLOAT: i64 = 1;

/// Tamanho em bytes de cada elemento.
const ELEM_SIZE_INT: i64 = 8;
const ELEM_SIZE_FLOAT: i64 = 8;

/// Layout do header do tensor na memória.
/// 48 bytes — 6 fields × 8 bytes cada.
const HEADER_SIZE: i64 = 48;

/// Offsets dentro do header.
const OFF_DATA: usize = 0;
const OFF_RANK: usize = 8;
const OFF_SHAPE: usize = 16;
const OFF_STRIDES: usize = 24;
const OFF_ELEM_TYPE: usize = 32;
const OFF_ELEM_SIZE: usize = 40;

// ── Helpers internos ──────────────────────────────────────────────

/// Aloca um tensor na arena: header + shape[rank] + strides[rank] + data[nelems * elem_size].
/// Retorna ponteiro para o header, ou 0 se falhar.
///
/// `data_in` é um ponteiro opcional para dados crus já prontos (copia para o buffer
/// alocado). Se 0, o buffer é zerado.
fn tensor_alloc(rank: i64, shape: &[i64], elem_type: i64, data_in: *const u8) -> i64 {
    assert!(rank >= 1, "rank deve ser >= 1");
    let elem_size = if elem_type == ELEM_INT {
        ELEM_SIZE_INT
    } else {
        ELEM_SIZE_FLOAT
    };
    let nelems: i64 = shape.iter().product();
    let shape_bytes = rank * 8;
    let strides_bytes = rank * 8;
    let data_bytes = nelems * elem_size;
    let total = HEADER_SIZE + shape_bytes + strides_bytes + data_bytes;

    let ptr = crate::arena::kata_rt_arena_alloc(crate::arena::rt_ptr(), 0, total);
    if ptr == 0 {
        return 0;
    }

    let base = ptr as *mut u8;
    let shape_ptr = unsafe { base.add(HEADER_SIZE as usize) } as *mut i64;
    let strides_ptr = unsafe { base.add((HEADER_SIZE + shape_bytes) as usize) } as *mut i64;
    let data_ptr = unsafe { base.add((HEADER_SIZE + shape_bytes + strides_bytes) as usize) };

    // Calcula strides row-major: strides[i] = product(shape[i+1..])
    let mut strides = vec![0i64; rank as usize];
    strides[(rank - 1) as usize] = 1;
    for i in (0..rank - 1).rev() {
        strides[i as usize] = strides[(i + 1) as usize] * shape[(i + 1) as usize];
    }

    // Preenche o header
    unsafe {
        std::ptr::write_unaligned(base.add(OFF_DATA) as *mut *mut u8, data_ptr);
        std::ptr::write_unaligned(base.add(OFF_RANK) as *mut i64, rank);
        std::ptr::write_unaligned(base.add(OFF_SHAPE) as *mut *mut i64, shape_ptr);
        std::ptr::write_unaligned(base.add(OFF_STRIDES) as *mut *mut i64, strides_ptr);
        std::ptr::write_unaligned(base.add(OFF_ELEM_TYPE) as *mut i64, elem_type);
        std::ptr::write_unaligned(base.add(OFF_ELEM_SIZE) as *mut i64, elem_size);

        // Copia shape
        for i in 0..rank as usize {
            std::ptr::write_unaligned(shape_ptr.add(i), shape[i]);
        }
        // Copia strides
        for i in 0..rank as usize {
            std::ptr::write_unaligned(strides_ptr.add(i), strides[i]);
        }
        // Copia ou zera data
        if data_in.is_null() {
            std::ptr::write_bytes(data_ptr, 0, data_bytes as usize);
        } else {
            std::ptr::copy_nonoverlapping(data_in, data_ptr, data_bytes as usize);
        }
    }

    ptr
}

/// Lê um field do header.
unsafe fn read_field(ptr: i64, offset: usize) -> i64 {
    unsafe { std::ptr::read_unaligned((ptr as *const u8).add(offset) as *const i64) }
}

/// Escreve um field no header.
unsafe fn write_field(ptr: i64, offset: usize, val: i64) {
    unsafe {
        std::ptr::write_unaligned((ptr as *mut u8).add(offset) as *mut i64, val);
    }
}

unsafe fn get_data_ptr(ptr: i64) -> *mut u8 {
    unsafe {
        let raw = read_field(ptr, OFF_DATA);
        raw as *mut u8
    }
}

unsafe fn get_rank(ptr: i64) -> i64 {
    unsafe { read_field(ptr, OFF_RANK) }
}

unsafe fn get_shape_ptr(ptr: i64) -> *mut i64 {
    unsafe {
        let raw = read_field(ptr, OFF_SHAPE);
        raw as *mut i64
    }
}

unsafe fn get_strides_ptr(ptr: i64) -> *mut i64 {
    unsafe {
        let raw = read_field(ptr, OFF_STRIDES);
        raw as *mut i64
    }
}

unsafe fn get_elem_type(ptr: i64) -> i64 {
    unsafe { read_field(ptr, OFF_ELEM_TYPE) }
}

unsafe fn get_elem_size(ptr: i64) -> i64 {
    unsafe { read_field(ptr, OFF_ELEM_SIZE) }
}

/// Lê shape como Vec<i64>.
unsafe fn get_shape_vec(ptr: i64) -> Vec<i64> {
    unsafe {
        let rank = get_rank(ptr) as usize;
        let shape_ptr = get_shape_ptr(ptr);
        (0..rank)
            .map(|i| std::ptr::read_unaligned(shape_ptr.add(i)))
            .collect()
    }
}

/// Lê strides como Vec<i64>.
unsafe fn get_strides_vec(ptr: i64) -> Vec<i64> {
    unsafe {
        let rank = get_rank(ptr) as usize;
        let strides_ptr = get_strides_ptr(ptr);
        (0..rank)
            .map(|i| std::ptr::read_unaligned(strides_ptr.add(i)))
            .collect()
    }
}

/// Número total de elementos (product de shape).
fn shape_nelems(shape: &[i64]) -> i64 {
    shape.iter().product()
}

/// Verifica se dois shapes são broadcastable (regras NumPy: right-aligned,
/// dims de tamanho 1 ou iguais). Retorna o shape resultante, ou None se incompatível.
fn broadcast_shapes(a: &[i64], b: &[i64]) -> Option<Vec<i64>> {
    let max_rank = a.len().max(b.len());
    let mut result = vec![0i64; max_rank];
    for i in 0..max_rank {
        let ai = if i < a.len() { a[a.len() - 1 - i] } else { 1 };
        let bi = if i < b.len() { b[b.len() - 1 - i] } else { 1 };
        if ai == bi {
            result[max_rank - 1 - i] = ai;
        } else if ai == 1 {
            result[max_rank - 1 - i] = bi;
        } else if bi == 1 {
            result[max_rank - 1 - i] = ai;
        } else {
            return None;
        }
    }
    Some(result)
}

/// Converte índice N-D em índice flatten (usando strides).
unsafe fn nd_to_flat(ptr: i64, indices: &[i64]) -> i64 {
    unsafe {
        let strides = get_strides_vec(ptr);
        let mut flat = 0i64;
        for (i, &idx) in indices.iter().enumerate() {
            flat += idx * strides[i];
        }
        flat
    }
}

/// Lê um elemento Int do tensor (no índice flatten).
unsafe fn read_elem_int(ptr: i64, flat: i64) -> i64 {
    unsafe {
        let data = get_data_ptr(ptr);
        let elem_size = get_elem_size(ptr);
        std::ptr::read_unaligned(data.add((flat * elem_size) as usize) as *const i64)
    }
}

/// Lê um elemento Float do tensor (no índice flatten).
unsafe fn read_elem_float(ptr: i64, flat: i64) -> f64 {
    unsafe {
        let data = get_data_ptr(ptr);
        let elem_size = get_elem_size(ptr);
        let bits = std::ptr::read_unaligned(data.add((flat * elem_size) as usize) as *const u64);
        f64::from_bits(bits)
    }
}

/// Escreve um elemento Int no tensor (no índice flatten).
unsafe fn write_elem_int(ptr: i64, flat: i64, val: i64) {
    unsafe {
        let data = get_data_ptr(ptr);
        let elem_size = get_elem_size(ptr);
        std::ptr::write_unaligned(data.add((flat * elem_size) as usize) as *mut i64, val);
    }
}

/// Escreve um elemento Float no tensor (no índice flatten).
unsafe fn write_elem_float(ptr: i64, flat: i64, val: f64) {
    unsafe {
        let data = get_data_ptr(ptr);
        let elem_size = get_elem_size(ptr);
        std::ptr::write_unaligned(
            data.add((flat * elem_size) as usize) as *mut u64,
            val.to_bits(),
        );
    }
}

// ── FFIs C-ABI ────────────────────────────────────────────────────

/// Cria um tensor a partir de dados crus. `data` é um ponteiro para os elementos
/// (i64 para Int, u64=bits de f64 para Float). `rank` ≥ 1. `shape` é ponteiro
/// para `rank` int64s.
///
/// `elem_type`: 0 = Int, 1 = Float.
///
/// # Safety
/// `data` deve ter `product(shape)` elementos válidos. `shape` deve ter `rank`
/// entradas. `rank` deve ser ≥ 1 (panic se 0).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_new(data: i64, rank: i64, shape: i64, elem_type: i64) -> i64 {
    if rank < 1 {
        panic!("kata_rt_tensor_new: rank deve ser >= 1, recebeu {rank}");
    }
    let shape_vec = unsafe {
        let s = shape as *const i64;
        (0..rank)
            .map(|i| std::ptr::read_unaligned(s.add(i as usize)))
            .collect::<Vec<_>>()
    };
    tensor_alloc(rank, &shape_vec, elem_type, data as *const u8)
}

/// Retorna o rank do tensor. SMI-tagged.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_rank(ptr: i64) -> i64 {
    if ptr == 0 {
        return tag_smi(0);
    }
    tag_smi(unsafe { get_rank(ptr) })
}

/// Retorna o shape como um ponteiro para `rank` int64s.
/// O caller não libera — vive na arena.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_shape(ptr: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    unsafe { get_shape_ptr(ptr) as i64 }
}

/// Acesso por índice flatten com bounds check. Retorna um Result box (Sum):
/// - Ok (tag=0): payload = valor do elemento (i64 SMI para Int, bits para Float)
/// - Err (tag=1): payload = ponteiro para C string com mensagem de erro
///
/// `idx` é SMI-tagged (vindo do codegen).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_at(ptr: i64, idx: i64) -> i64 {
    let idx = untag_smi(idx);
    if ptr == 0 {
        return crate::sum::err_with_msg("at: tensor nulo", 0);
    }
    let nelems = unsafe { get_shape_vec(ptr) }.iter().product::<i64>();
    if idx < 0 || idx >= nelems {
        return crate::sum::err_with_msg("at: índice fora dos limites", 0);
    }
    let elem_type = unsafe { get_elem_type(ptr) };
    let val = unsafe {
        if elem_type == ELEM_INT {
            read_elem_int(ptr, idx)
        } else {
            read_elem_float(ptr, idx).to_bits() as i64 // bits brutas
        }
    };
    crate::sum::kata_rt_store_sum_result(0, val, 0)
}

/// Adição element-wise com broadcast. Retorna Result box:
/// - Ok (tag=0): payload = ponteiro para novo tensor
/// - Err (tag=1): payload = mensagem de erro
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_add(a: i64, b: i64) -> i64 {
    tensor_elementwise(a, b, true)
}

/// Multiplicação element-wise (Hadamard) com broadcast. Retorna Result box.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_mul(a: i64, b: i64) -> i64 {
    tensor_elementwise(a, b, false)
}

/// Variante panic de add. Retorna tensor* direto (sem Result).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_panic_add(a: i64, b: i64) -> i64 {
    let result = tensor_elementwise(a, b, true);
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    if tag == 1 {
        let msg_ptr =
            unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
        let msg = unsafe {
            std::ffi::CStr::from_ptr(msg_ptr as *const std::os::raw::c_char)
                .to_string_lossy()
                .into_owned()
        };
        panic!("kata_rt_tensor_panic_add: {msg}");
    }
    unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) }
}

/// Variante panic de mul. Retorna tensor* direto (sem Result).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_panic_mul(a: i64, b: i64) -> i64 {
    let result = tensor_elementwise(a, b, false);
    let tag = unsafe { std::ptr::read_unaligned(result as *const i64) };
    if tag == 1 {
        let msg_ptr =
            unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) };
        let msg = unsafe {
            std::ffi::CStr::from_ptr(msg_ptr as *const std::os::raw::c_char)
                .to_string_lossy()
                .into_owned()
        };
        panic!("kata_rt_tensor_panic_mul: {msg}");
    }
    unsafe { std::ptr::read_unaligned((result as *const u8).add(8) as *const i64) }
}

/// Núcleo de add/mul element-wise com broadcast.
/// `is_add`: true = adição, false = multiplicação (Hadamard).
fn tensor_elementwise(a: i64, b: i64, is_add: bool) -> i64 {
    if a == 0 || b == 0 {
        return crate::sum::err_with_msg(
            if is_add {
                "add: tensor nulo"
            } else {
                "mul: tensor nulo"
            },
            0,
        );
    }
    let a_shape = unsafe { get_shape_vec(a) };
    let b_shape = unsafe { get_shape_vec(b) };
    let out_shape = match broadcast_shapes(&a_shape, &b_shape) {
        Some(s) => s,
        None => {
            return crate::sum::err_with_msg(
                if is_add {
                    "add: shapes não broadcastable"
                } else {
                    "mul: shapes não broadcastable"
                },
                0,
            );
        }
    };
    let elem_type = unsafe { get_elem_type(a) };
    if elem_type != unsafe { get_elem_type(b) } {
        return crate::sum::err_with_msg(
            if is_add {
                "add: tipos de elemento diferentes"
            } else {
                "mul: tipos de elemento diferentes"
            },
            0,
        );
    }

    let result = tensor_alloc(
        out_shape.len() as i64,
        &out_shape,
        elem_type,
        std::ptr::null(),
    );
    if result == 0 {
        return crate::sum::err_with_msg("tensor: falha de alocação", 0);
    }

    let out_nelems = shape_nelems(&out_shape);
    let out_rank = out_shape.len();

    // Para cada índice no tensor de saída, computa os índices correspondentes
    // em a e b (broadcast), aplica a operação.
    let _a_rank = a_shape.len();
    let _b_rank = b_shape.len();

    for flat in 0..out_nelems {
        // Decompõe flat em índices N-D no output
        let mut out_idx = vec![0i64; out_rank];
        let mut rem = flat;
        for i in (0..out_rank).rev() {
            out_idx[i] = rem % out_shape[i];
            rem /= out_shape[i];
        }

        // Mapeia para índices em a (right-aligned, broadcast)
        let a_flat = broadcast_flat(&out_idx, &out_shape, &a_shape, a);
        let b_flat = broadcast_flat(&out_idx, &out_shape, &b_shape, b);

        unsafe {
            if elem_type == ELEM_INT {
                let av = untag_smi(read_elem_int(a, a_flat));
                let bv = untag_smi(read_elem_int(b, b_flat));
                let rv = if is_add { av + bv } else { av * bv };
                write_elem_int(result, flat, tag_smi(rv));
            } else {
                let av = read_elem_float(a, a_flat);
                let bv = read_elem_float(b, b_flat);
                let rv = if is_add { av + bv } else { av * bv };
                write_elem_float(result, flat, rv);
            }
        }
    }

    crate::sum::kata_rt_store_sum_result(0, result, 0)
}

/// Mapeia um índice N-D do output para um índice flatten no tensor `ptr`,
/// aplicando broadcast (right-aligned, dims de tamanho 1 são indexadas como 0).
fn broadcast_flat(out_idx: &[i64], out_shape: &[i64], tensor_shape: &[i64], ptr: i64) -> i64 {
    let tensor_rank = tensor_shape.len();
    let offset = out_shape.len() - tensor_rank;
    let strides = unsafe { get_strides_vec(ptr) };
    let mut flat = 0i64;
    for i in 0..tensor_rank {
        let oi = i + offset;
        // Se a dim do tensor é 1 e a dim do output > 1, sempre índice 0 (broadcast)
        let idx = if tensor_shape[i] == 1 { 0 } else { out_idx[oi] };
        flat += idx * strides[i];
    }
    flat
}

/// Contração (dot product). Para 2D×2D: matmul. Para 1D×1D: produto escalar.
/// N-D segue convenção NumPy (último de A × penúltimo de B).
///
/// Retorna Result box: Ok = novo tensor, Err = mensagem.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_dot(a: i64, b: i64) -> i64 {
    if a == 0 || b == 0 {
        return crate::sum::err_with_msg("dot: tensor nulo", 0);
    }
    let a_shape = unsafe { get_shape_vec(a) };
    let b_shape = unsafe { get_shape_vec(b) };
    let a_rank = a_shape.len();
    let b_rank = b_shape.len();
    let elem_type = unsafe { get_elem_type(a) };
    if elem_type != unsafe { get_elem_type(b) } {
        return crate::sum::err_with_msg("dot: tipos de elemento diferentes", 0);
    }

    // 1D × 1D → produto escalar (0-D não existe, retorna 1-element tensor)
    if a_rank == 1 && b_rank == 1 {
        if a_shape[0] != b_shape[0] {
            return crate::sum::err_with_msg("dot: dimensões internas não casam", 0);
        }
        let n = a_shape[0];
        let result_shape = vec![1i64];
        let result = tensor_alloc(1, &result_shape, elem_type, std::ptr::null());
        if result == 0 {
            return crate::sum::err_with_msg("dot: falha de alocação", 0);
        }
        unsafe {
            if elem_type == ELEM_INT {
                let mut sum = 0i64;
                for i in 0..n {
                    sum += untag_smi(read_elem_int(a, i)) * untag_smi(read_elem_int(b, i));
                }
                write_elem_int(result, 0, tag_smi(sum));
            } else {
                let mut sum = 0f64;
                for i in 0..n {
                    sum += read_elem_float(a, i) * read_elem_float(b, i);
                }
                write_elem_float(result, 0, sum);
            }
        }
        return crate::sum::kata_rt_store_sum_result(0, result, 0);
    }

    // 2D × 2D → matmul (m×k) · (k×n) = (m×n)
    if a_rank == 2 && b_rank == 2 {
        let m = a_shape[0];
        let k = a_shape[1];
        let k2 = b_shape[0];
        let n = b_shape[1];
        if k != k2 {
            return crate::sum::err_with_msg("dot: dimensões internas não casam", 0);
        }
        let result_shape = vec![m, n];
        let result = tensor_alloc(2, &result_shape, elem_type, std::ptr::null());
        if result == 0 {
            return crate::sum::err_with_msg("dot: falha de alocação", 0);
        }

        unsafe {
            if elem_type == ELEM_FLOAT {
                // matrixmultiply: dgemm(m, k, n, alpha, a, rsa, csa, b, rsb, csb, beta, c, rsc, csc)
                // Row-major A (m×k): rsa=k, csa=1. Row-major B (k×n): rsb=n, csb=1.
                // Row-major C (m×n): rsc=n, csc=1.
                let a_data = get_data_ptr(a) as *const f64;
                let b_data = get_data_ptr(b) as *const f64;
                let c_data = get_data_ptr(result) as *mut f64;
                matrixmultiply::dgemm(
                    m as usize, k as usize, n as usize, 1.0, a_data, k as isize, 1, b_data,
                    n as isize, 1, 0.0, c_data, n as isize, 1,
                );
            } else {
                // Int: loop próprio (SMI-tagged)
                for i in 0..m {
                    for j in 0..n {
                        let mut sum = 0i64;
                        for l in 0..k {
                            let av = untag_smi(read_elem_int(a, i * k + l));
                            let bv = untag_smi(read_elem_int(b, l * n + j));
                            sum += av * bv;
                        }
                        write_elem_int(result, i * n + j, tag_smi(sum));
                    }
                }
            }
        }
        return crate::sum::kata_rt_store_sum_result(0, result, 0);
    }

    // 2D × 1D → matmul com N=1
    if a_rank == 2 && b_rank == 1 {
        let m = a_shape[0];
        let k = a_shape[1];
        if k != b_shape[0] {
            return crate::sum::err_with_msg("dot: dimensões internas não casam", 0);
        }
        let result_shape = vec![m];
        let result = tensor_alloc(1, &result_shape, elem_type, std::ptr::null());
        if result == 0 {
            return crate::sum::err_with_msg("dot: falha de alocação", 0);
        }
        unsafe {
            if elem_type == ELEM_FLOAT {
                // dgemm com n=1: A (m×k) row-major × B (k×1) → C (m×1)
                // Row-major A (m×k): rsa=k, csa=1. B como (k×1): rsb=1, csb=k.
                // C como (m×1): rsc=1, csc=m.
                let a_data = get_data_ptr(a) as *const f64;
                let b_data = get_data_ptr(b) as *const f64;
                let c_data = get_data_ptr(result) as *mut f64;
                matrixmultiply::dgemm(
                    m as usize, k as usize, 1, 1.0, a_data, k as isize, 1, b_data, 1, k as isize,
                    0.0, c_data, 1, m as isize,
                );
            } else {
                for i in 0..m {
                    let mut sum = 0i64;
                    for l in 0..k {
                        sum +=
                            untag_smi(read_elem_int(a, i * k + l)) * untag_smi(read_elem_int(b, l));
                    }
                    write_elem_int(result, i, tag_smi(sum));
                }
            }
        }
        return crate::sum::kata_rt_store_sum_result(0, result, 0);
    }

    // TODO: N-D general (convenção NumPy: último de A × penúltimo de B)
    crate::sum::err_with_msg("dot: rank combination not yet supported", 0)
}

/// Transposição 2-D (swap dos dois eixos). Zero-copy: troca strides no header.
/// Para N-D, transpõe os dois últimos eixos (convenção NumPy).
///
/// Retorna novo header com mesmo data buffer (não copia dados).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_transpose(ptr: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    let rank = unsafe { get_rank(ptr) };
    if rank < 2 {
        // 1-D: transposição é identidade — retorna cópia do header
        let shape = unsafe { get_shape_vec(ptr) };
        let elem_type = unsafe { get_elem_type(ptr) };
        let data = unsafe { get_data_ptr(ptr) };
        return tensor_alloc(rank, &shape, elem_type, data);
    }

    let shape = unsafe { get_shape_vec(ptr) };
    let strides = unsafe { get_strides_vec(ptr) };
    let elem_type = unsafe { get_elem_type(ptr) };
    let data = unsafe { get_data_ptr(ptr) };

    // Transpõe os dois últimos eixos
    let mut new_shape = shape.clone();
    let mut new_strides = strides.clone();
    let last = (rank as usize) - 1;
    let penult = last - 1;
    new_shape.swap(last, penult);
    new_strides.swap(last, penult);

    // Aloca novo header com shape transposto, copia data
    let result = tensor_alloc(rank, &new_shape, elem_type, data);
    if result == 0 {
        return 0;
    }

    // Sobrescreve strides com os transpostos (tensor_alloc calcula row-major;
    // precisamos dos strides transpostos para zero-copy view)
    unsafe {
        let strides_ptr = get_strides_ptr(result);
        for i in 0..rank as usize {
            std::ptr::write_unaligned(strides_ptr.add(i), new_strides[i]);
        }
    }

    result
}

/// Multiplica todos os elementos por um escalar. Retorna novo tensor.
/// `scalar` é o valor bruto (i64 para Int, bits de f64 para Float).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_scale(ptr: i64, scalar: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    let shape = unsafe { get_shape_vec(ptr) };
    let elem_type = unsafe { get_elem_type(ptr) };
    let result = tensor_alloc(shape.len() as i64, &shape, elem_type, std::ptr::null());
    if result == 0 {
        return 0;
    }
    let nelems = shape_nelems(&shape);
    unsafe {
        if elem_type == ELEM_INT {
            let s = untag_smi(scalar);
            for i in 0..nelems {
                write_elem_int(result, i, tag_smi(untag_smi(read_elem_int(ptr, i)) * s));
            }
        } else {
            let s = f64::from_bits(scalar as u64);
            for i in 0..nelems {
                write_elem_float(result, i, read_elem_float(ptr, i) * s);
            }
        }
    }
    result
}

/// Soma um escalar a cada elemento. Retorna novo tensor.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_shift(ptr: i64, scalar: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    let shape = unsafe { get_shape_vec(ptr) };
    let elem_type = unsafe { get_elem_type(ptr) };
    let result = tensor_alloc(shape.len() as i64, &shape, elem_type, std::ptr::null());
    if result == 0 {
        return 0;
    }
    let nelems = shape_nelems(&shape);
    unsafe {
        if elem_type == ELEM_INT {
            let s = untag_smi(scalar);
            for i in 0..nelems {
                write_elem_int(result, i, tag_smi(untag_smi(read_elem_int(ptr, i)) + s));
            }
        } else {
            let s = f64::from_bits(scalar as u64);
            for i in 0..nelems {
                write_elem_float(result, i, read_elem_float(ptr, i) + s);
            }
        }
    }
    result
}

/// Indexação N-D escalar — todos os eixos como Int.
/// `indices_ptr` aponta para `n_indices` int64s (SMI-tagged).
/// Retorna Result box: Ok = elemento (SMI-tagged Int ou bits de Float),
/// Err = mensagem de erro (out-of-bounds).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_at_nd(ptr: i64, indices_ptr: i64, n_indices: i64) -> i64 {
    if ptr == 0 {
        return crate::sum::err_with_msg("at_nd: tensor nulo", 0);
    }
    let rank = unsafe { get_rank(ptr) };
    if n_indices != rank {
        return crate::sum::err_with_msg("at_nd: número de índices != rank", 0);
    }
    let shape = unsafe { get_shape_vec(ptr) };
    let strides = unsafe { get_strides_vec(ptr) };
    let elem_type = unsafe { get_elem_type(ptr) };

    // Lê índices e valida bounds
    let mut flat: i64 = 0;
    for i in 0..n_indices as usize {
        let idx_smi = unsafe {
            std::ptr::read_unaligned((indices_ptr as *const u8).add(i * 8) as *const i64)
        };
        let idx = untag_smi(idx_smi);
        if idx < 0 || idx >= shape[i] {
            return crate::sum::err_with_msg("at_nd: índice fora dos limites", 0);
        }
        flat += idx * strides[i];
    }

    let val = unsafe {
        if elem_type == ELEM_INT {
            read_elem_int(ptr, flat)
        } else {
            read_elem_float(ptr, flat).to_bits() as i64
        }
    };
    crate::sum::kata_rt_store_sum_result(0, val, 0)
}

/// Sub-tensor N-D — eixos podem ser Int (colapsa), Range (preserva), ou Wildcard.
/// `starts_ptr` e `ends_ptr` apontam para `n_axes` int64s cada.
/// Para eixos Int: start = idx, end = idx+1 (será colapsado).
/// Para eixos Wildcard: start = 0, end = shape[axis].
/// Para eixos Range: start e end conforme especificado (end exclusive).
/// `collapse_mask` — bitmask: bit i = 1 se eixo i deve ser colapsado (Int).
/// Retorna ponteiro para novo tensor (sub-tensor). Retorna 0 se falhar.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_sub(
    ptr: i64,
    starts_ptr: i64,
    ends_ptr: i64,
    n_axes: i64,
    collapse_mask: i64,
) -> i64 {
    if ptr == 0 {
        return 0;
    }
    let shape = unsafe { get_shape_vec(ptr) };
    let strides = unsafe { get_strides_vec(ptr) };
    let elem_type = unsafe { get_elem_type(ptr) };

    if n_axes as usize != shape.len() {
        return 0;
    }

    // Lê starts e ends, calcula novo shape e offset inicial
    let mut new_shape: Vec<i64> = Vec::new();
    let mut offset_flat: i64 = 0;

    for i in 0..n_axes as usize {
        let start = untag_smi(unsafe {
            std::ptr::read_unaligned((starts_ptr as *const u8).add(i * 8) as *const i64)
        });
        let mut end = untag_smi(unsafe {
            std::ptr::read_unaligned((ends_ptr as *const u8).add(i * 8) as *const i64)
        });

        // Sentinela -1 = Wildcard: seleciona todas as posições do eixo
        if end == -1 {
            end = shape[i];
        }

        let is_collapsed = (collapse_mask >> i) & 1 == 1;

        // Valida bounds
        if start < 0 || end > shape[i] || start > end {
            return 0;
        }

        offset_flat += start * strides[i];

        if !is_collapsed {
            new_shape.push(end - start);
        }
    }

    // Se todos os eixos foram colapsados e new_shape está vazio, isso
    // seria um escalar — mas tensores 0-D não existem. Retorna erro (0).
    if new_shape.is_empty() {
        return 0;
    }

    // Aloca o novo tensor
    let new_rank = new_shape.len() as i64;
    let result = tensor_alloc(new_rank, &new_shape, elem_type, std::ptr::null());
    if result == 0 {
        return 0;
    }

    // Copia dados: itera sobre todas as posições do novo tensor,
    // mapeia cada uma para a posição correspondente no tensor original.
    let new_nelems = shape_nelems(&new_shape);
    let new_strides = {
        let mut s = vec![0i64; new_rank as usize];
        s[(new_rank - 1) as usize] = 1;
        for i in (0..new_rank - 1).rev() {
            s[i as usize] = s[(i + 1) as usize] * new_shape[(i + 1) as usize];
        }
        s
    };

    // Constrói lista de strides originais para eixos não-colapsados
    let mut orig_strides: Vec<i64> = Vec::new();
    for i in 0..n_axes as usize {
        let is_collapsed = (collapse_mask >> i) & 1 == 1;
        if !is_collapsed {
            orig_strides.push(strides[i]);
        }
    }

    for new_flat in 0..new_nelems {
        // Decompõe new_flat em coords N-D e mapeia para o tensor original
        let mut remaining = new_flat;
        let mut src_flat = offset_flat;
        for i in 0..new_rank as usize {
            let c = remaining / new_strides[i];
            remaining %= new_strides[i];
            src_flat += c * orig_strides[i];
        }

        unsafe {
            if elem_type == ELEM_INT {
                let val = read_elem_int(ptr, src_flat);
                write_elem_int(result, new_flat, val);
            } else {
                let val = read_elem_float(ptr, src_flat);
                write_elem_float(result, new_flat, val);
            }
        }
    }

    result
}

/// Libera o tensor. No-op para arenas bump (liberação é bulk).
/// Mantém a assinatura para compatibilidade AOT.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_free(_ptr: i64) {
    // Arena bump: liberação é bulk via arena_destroy.
    // Tracked arena: não rastreamos tensors individualmente ainda.
}

/// Constrói um Text (C string) com a representação de display do tensor.
/// Retorna ponteiro para CString (owned, via into_raw).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tensor_show(ptr: i64) -> i64 {
    if ptr == 0 {
        let s = CString::new("<tensor nulo>").unwrap();
        return s.into_raw() as i64;
    }
    let shape = unsafe { get_shape_vec(ptr) };
    let elem_type = unsafe { get_elem_type(ptr) };
    let s = format_tensor(ptr, &shape, elem_type);
    let cstr = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
    cstr.into_raw() as i64
}

/// Formata o tensor como string tabular.
fn format_tensor(ptr: i64, shape: &[i64], elem_type: i64) -> String {
    let rank = shape.len();
    let nelems = shape_nelems(shape);
    if nelems == 0 {
        return String::new();
    }

    // Formata todos os elementos como strings
    let mut cells: Vec<String> = Vec::with_capacity(nelems as usize);
    for flat in 0..nelems {
        let s = unsafe {
            if elem_type == ELEM_INT {
                let v = read_elem_int(ptr, flat);
                // SMI untagging para display
                let untagged = untag_smi(v);
                untagged.to_string()
            } else {
                let v = read_elem_float(ptr, flat);
                format!("{}", v)
            }
        };
        cells.push(s);
    }

    if rank == 1 {
        // 1-D: uma linha, espaços entre colunas
        let max_width = cells.iter().map(|s| s.len()).max().unwrap_or(0);
        let padded: Vec<String> = cells
            .iter()
            .map(|s| format!("{:>width$}", s, width = max_width))
            .collect();
        return padded.join("  ");
    }

    if rank == 2 {
        let rows = shape[0] as usize;
        let cols = shape[1] as usize;
        // Largura de cada coluna
        let mut col_widths = vec![0usize; cols];
        for r in 0..rows {
            for c in 0..cols {
                let cell = &cells[r * cols + c];
                col_widths[c] = col_widths[c].max(cell.len());
            }
        }
        let mut lines = Vec::new();
        for r in 0..rows {
            let row_cells: Vec<String> = (0..cols)
                .map(|c| format!("{:>width$}", &cells[r * cols + c], width = col_widths[c]))
                .collect();
            lines.push(row_cells.join("  "));
        }
        return lines.join("\n");
    }

    // Rank > 2: fatias 2-D ao longo do eixo 0
    let n_slices = shape[0] as usize;
    let slice_shape = &shape[1..];
    let slice_nelems = shape_nelems(slice_shape);
    let mut parts = Vec::new();
    for s in 0..n_slices {
        let start = (s as i64) * slice_nelems;
        let slice_cells: Vec<String> =
            cells[start as usize..(start + slice_nelems) as usize].to_vec();
        let slice_str = format_2d_or_deeper(&slice_cells, slice_shape, elem_type);
        if n_slices > 1 {
            parts.push(format!("[{}]:\n{}", s, slice_str));
        } else {
            parts.push(slice_str);
        }
    }
    parts.join("\n\n")
}

/// Formata uma fatia (rank-1 ou rank-2) a partir de células já convertidas em string.
fn format_2d_or_deeper(cells: &[String], shape: &[i64], _elem_type: i64) -> String {
    let rank = shape.len();
    if rank == 1 {
        let max_width = cells.iter().map(|s| s.len()).max().unwrap_or(0);
        let padded: Vec<String> = cells
            .iter()
            .map(|s| format!("{:>width$}", s, width = max_width))
            .collect();
        return padded.join("  ");
    }
    // rank == 2
    let rows = shape[0] as usize;
    let cols = shape[1] as usize;
    let mut col_widths = vec![0usize; cols];
    for r in 0..rows {
        for c in 0..cols {
            let cell = &cells[r * cols + c];
            col_widths[c] = col_widths[c].max(cell.len());
        }
    }
    let mut lines = Vec::new();
    for r in 0..rows {
        let row_cells: Vec<String> = (0..cols)
            .map(|c| format!("{:>width$}", &cells[r * cols + c], width = col_widths[c]))
            .collect();
        lines.push(row_cells.join("  "));
    }
    lines.join("\n")
}
