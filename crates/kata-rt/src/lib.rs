//! Runtime isolado da Kata-Lang.
//!
//! BigInt/SMI tagging, Float, Rational, Text, arena, print.
//! Linkada via symbol map (C-ABI). Desconhece as regras internas da linguagem.
//!
//! O compilador conhece apenas o enum `FfiSymbol` e as 3 strings de mapeamento
//! (`"i64"`, `"f64"`, `"kata_rt_string"`). Toda a implementação vive aqui.

pub mod arena;
pub mod bigint;
pub mod float;
pub mod io;
pub mod rational;
pub mod text;

// Re-exports convenientes para uso interno (não C-ABI)
pub use bigint::{
    bigint_to_string, decode_smi_pub, encode_smi_pub, fits_smi_pub, is_smi_pub, show,
    tag_int_from_str, tag_int_pub, to_rational,
};
pub use float::float_to_string;
pub use rational::{float_to_rat, rat_from_int, rat_from_text, rat_to_float, rat_to_string};
pub use text::{bool_to_text, int_to_text, text_literal, text_replace_first};
