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

// Re-exports de funções C-ABI para o codegen registrar no JIT.
pub use bigint::{
    kata_rt_bi_add, kata_rt_bi_div, kata_rt_bi_eq, kata_rt_bi_ge, kata_rt_bi_gt, kata_rt_bi_le,
    kata_rt_bi_lt, kata_rt_bi_mul, kata_rt_bi_neq, kata_rt_bi_show, kata_rt_bi_sub,
    kata_rt_bi_to_rational, kata_rt_int_to_text, kata_rt_tag_int, kata_rt_tag_int_from_str,
};
pub use float::{
    kata_rt_fadd, kata_rt_fcmp_eq, kata_rt_fcmp_ge, kata_rt_fcmp_gt, kata_rt_fcmp_le,
    kata_rt_fcmp_lt, kata_rt_fcmp_neq, kata_rt_fdiv, kata_rt_fmul, kata_rt_fsub,
};
pub use io::{kata_rt_print, kata_rt_println};
pub use rational::{
    kata_rt_int_to_rational, kata_rt_rat_add, kata_rt_rat_div, kata_rt_rat_eq,
    kata_rt_rat_from_float, kata_rt_rat_ge, kata_rt_rat_gt, kata_rt_rat_le, kata_rt_rat_literal,
    kata_rt_rat_lt, kata_rt_rat_mul, kata_rt_rat_neq, kata_rt_rat_show, kata_rt_rat_show_raw,
    kata_rt_rat_sub, kata_rt_rat_to_float,
};
pub use text::{
    kata_rt_bool_to_text, kata_rt_string_concat, kata_rt_string_len, kata_rt_text_literal,
    kata_rt_text_replace_first,
};
