//! Runtime isolado da Kata-Lang.
//!
//! BigInt/SMI tagging, Float, Rational, Text, arena, print.
//! Linkada via symbol map (C-ABI). Desconhece as regras internas da linguagem.
//!
//! O compilador conhece apenas o enum `FfiSymbol` e as 3 strings de mapeamento
//! (`"i64"`, `"f64"`, `"kata_rt_string"`). Toda a implementação vive aqui.

// Implementação vem no Fio 1.
