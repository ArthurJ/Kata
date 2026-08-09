//! Assinaturas FFI para File I/O e Socket I/O.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de file e socket I/O.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(CallConv::SystemV);
    match sym {
        // ── File I/O ──
        // file_open: (path_ptr, mode_tag) -> i64 (Result box ARC)
        FfiSymbol::FileOpen => {
            sig.params.push(AbiParam::new(I64)); // path_ptr (Text)
            sig.params.push(AbiParam::new(I64)); // mode_tag (FileMode variant tag)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_read: (handle) -> i64 (Result box ARC)
        FfiSymbol::FileRead => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_read_chunk: (handle, n) -> i64 (Result box)
        FfiSymbol::FileReadChunk => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // n (SMI-tagged)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_readline: (handle) -> i64 (Result box ARC)
        FfiSymbol::FileReadline => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_write_text: (handle, data_ptr) -> i64 (Result box ARC)
        FfiSymbol::FileWriteText => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // data_ptr (Text — C string)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_write_bytes: (handle, data_ptr) -> i64 (Result box ARC)
        FfiSymbol::FileWriteBytes => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // data_ptr (Bytes — blob with len header)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // file_close: (handle) -> void
        FfiSymbol::FileClose => {
            sig.params.push(AbiParam::new(I64)); // handle
        }
        // ── Socket I/O ──
        // socket_open: (kind_box, mode_box) -> i64 (Result box)
        FfiSymbol::SocketOpen => {
            sig.params.push(AbiParam::new(I64)); // kind_box (SocketKind Sum box)
            sig.params.push(AbiParam::new(I64)); // mode_box (SocketMode Sum box)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_listen: (listener_handle) -> i64 (Result box)
        FfiSymbol::SocketListen => {
            sig.params.push(AbiParam::new(I64)); // listener_handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_read: (handle) -> i64 (Result box)
        FfiSymbol::SocketRead => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_read_chunk: (handle, n) -> i64 (Result box)
        FfiSymbol::SocketReadChunk => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // n (SMI-tagged)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_readline: (handle) -> i64 (Result box)
        FfiSymbol::SocketReadline => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_write_text: (handle, data_ptr) -> i64 (Result box)
        FfiSymbol::SocketWriteText => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // data_ptr (Text — C string)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_write_bytes: (handle, data_ptr) -> i64 (Result box)
        FfiSymbol::SocketWriteBytes => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // data_ptr (Bytes — blob)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // socket_close: (handle) -> void
        FfiSymbol::SocketClose => {
            sig.params.push(AbiParam::new(I64)); // handle
        }
        _ => return None,
    }
    Some(sig)
}
