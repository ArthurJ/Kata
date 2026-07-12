//! `FfiSymbol` — enum tipado de símbolos FFI.
//!
//! Substitui strings soltas — se você errar o símbolo, é erro de compilação
//! do compilador, não bug silencioso em runtime. Cada variante carrega
//! metadados: `symbol_name()`, `return_type()`, etc.

use crate::ty::Ty;

/// Símbolo FFI catalogado. O compilador conhece apenas isto e as 3 strings
/// de mapeamento de representação (`"i64"`, `"f64"`, `"kata_rt_string"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FfiSymbol {
    // ── BigInt (Int com SMI tagging) ─────────────────────
    BiAdd,
    BiSub,
    BiMul,
    BiDiv,
    BiEq,
    BiNeq,
    BiLt,
    BiLe,
    BiGt,
    BiGe,
    BiShow,
    BiToRational,
    TagInt,

    // ── Float ────────────────────────────────────────────
    Fadd,
    Fsub,
    Fmul,
    Fdiv,
    FcmpEq,
    FcmpNeq,
    FcmpLt,
    FcmpLe,
    FcmpGt,
    FcmpGe,

    // ── Rational ─────────────────────────────────────────
    RatAdd,
    RatSub,
    RatMul,
    RatDiv,
    RatEq,
    RatNeq,
    RatLt,
    RatLe,
    RatGt,
    RatGe,
    RatShow,
    RatToFloat,
    RatFromFloat,
    RatLiteral,
    IntToRational,

    // ── Text ─────────────────────────────────────────────
    StringConcat,
    StringLen,
    TextLiteral,
    IntToText,
    BoolToText,
    TextReplaceFirst,

    // ── I/O ──────────────────────────────────────────────
    Print,
    Println,

    // ── Arena ────────────────────────────────────────────
    ArenaCreate,
    ArenaAlloc,
    ArenaDestroy,

    // ── Sum (Fase 5) ──────────────────────────────────────
    /// `kata_rt_store_sum_result(tag, payload) -> ptr` — aloca box Sum.
    StoreSumResult,
    /// `kata_rt_sum_tag_int(val) -> tag` — extrai tag de Sum box.
    SumTagInt,
}

impl FfiSymbol {
    /// Nome do símbolo C no runtime.
    pub fn symbol_name(self) -> &'static str {
        match self {
            FfiSymbol::BiAdd => "kata_rt_bi_add",
            FfiSymbol::BiSub => "kata_rt_bi_sub",
            FfiSymbol::BiMul => "kata_rt_bi_mul",
            FfiSymbol::BiDiv => "kata_rt_bi_div",
            FfiSymbol::BiEq => "kata_rt_bi_eq",
            FfiSymbol::BiNeq => "kata_rt_bi_neq",
            FfiSymbol::BiLt => "kata_rt_bi_lt",
            FfiSymbol::BiLe => "kata_rt_bi_le",
            FfiSymbol::BiGt => "kata_rt_bi_gt",
            FfiSymbol::BiGe => "kata_rt_bi_ge",
            FfiSymbol::BiShow => "kata_rt_bi_show",
            FfiSymbol::BiToRational => "kata_rt_bi_to_rational",
            FfiSymbol::TagInt => "kata_rt_tag_int",
            FfiSymbol::Fadd => "kata_rt_fadd",
            FfiSymbol::Fsub => "kata_rt_fsub",
            FfiSymbol::Fmul => "kata_rt_fmul",
            FfiSymbol::Fdiv => "kata_rt_fdiv",
            FfiSymbol::FcmpEq => "kata_rt_fcmp_eq",
            FfiSymbol::FcmpNeq => "kata_rt_fcmp_neq",
            FfiSymbol::FcmpLt => "kata_rt_fcmp_lt",
            FfiSymbol::FcmpLe => "kata_rt_fcmp_le",
            FfiSymbol::FcmpGt => "kata_rt_fcmp_gt",
            FfiSymbol::FcmpGe => "kata_rt_fcmp_ge",
            FfiSymbol::RatAdd => "kata_rt_rat_add",
            FfiSymbol::RatSub => "kata_rt_rat_sub",
            FfiSymbol::RatMul => "kata_rt_rat_mul",
            FfiSymbol::RatDiv => "kata_rt_rat_div",
            FfiSymbol::RatEq => "kata_rt_rat_eq",
            FfiSymbol::RatNeq => "kata_rt_rat_neq",
            FfiSymbol::RatLt => "kata_rt_rat_lt",
            FfiSymbol::RatLe => "kata_rt_rat_le",
            FfiSymbol::RatGt => "kata_rt_rat_gt",
            FfiSymbol::RatGe => "kata_rt_rat_ge",
            FfiSymbol::RatShow => "kata_rt_rat_show",
            FfiSymbol::RatToFloat => "kata_rt_rat_to_float",
            FfiSymbol::RatFromFloat => "kata_rt_rat_from_float",
            FfiSymbol::RatLiteral => "kata_rt_rat_literal",
            FfiSymbol::IntToRational => "kata_rt_int_to_rational",
            FfiSymbol::StringConcat => "kata_rt_string_concat",
            FfiSymbol::StringLen => "kata_rt_string_len",
            FfiSymbol::TextLiteral => "kata_rt_text_literal",
            FfiSymbol::IntToText => "kata_rt_int_to_text",
            FfiSymbol::BoolToText => "kata_rt_bool_to_text",
            FfiSymbol::TextReplaceFirst => "kata_rt_text_replace_first",
            FfiSymbol::Print => "kata_rt_print",
            FfiSymbol::Println => "kata_rt_println",
            FfiSymbol::ArenaCreate => "kata_rt_arena_create",
            FfiSymbol::ArenaAlloc => "kata_rt_arena_alloc",
            FfiSymbol::ArenaDestroy => "kata_rt_arena_destroy",
            FfiSymbol::StoreSumResult => "kata_rt_store_sum_result",
            FfiSymbol::SumTagInt => "kata_rt_sum_tag_int",
        }
    }

    /// Tipo de retorno do símbolo FFI.
    pub fn return_type(self) -> Ty {
        match self {
            // Aritmética Int → Int
            FfiSymbol::BiAdd | FfiSymbol::BiSub | FfiSymbol::BiMul | FfiSymbol::BiDiv => Ty::int(),
            // Comparação Int → Boolean
            FfiSymbol::BiEq
            | FfiSymbol::BiNeq
            | FfiSymbol::BiLt
            | FfiSymbol::BiLe
            | FfiSymbol::BiGt
            | FfiSymbol::BiGe => Ty::boolean(),
            FfiSymbol::BiShow | FfiSymbol::BiToRational => Ty::text(),
            FfiSymbol::TagInt => Ty::int(),
            // Float
            FfiSymbol::Fadd | FfiSymbol::Fsub | FfiSymbol::Fmul | FfiSymbol::Fdiv => Ty::float(),
            FfiSymbol::FcmpEq
            | FfiSymbol::FcmpNeq
            | FfiSymbol::FcmpLt
            | FfiSymbol::FcmpLe
            | FfiSymbol::FcmpGt
            | FfiSymbol::FcmpGe => Ty::boolean(),
            // Rational
            FfiSymbol::RatAdd | FfiSymbol::RatSub | FfiSymbol::RatMul | FfiSymbol::RatDiv => {
                Ty::rational()
            }
            FfiSymbol::RatEq
            | FfiSymbol::RatNeq
            | FfiSymbol::RatLt
            | FfiSymbol::RatLe
            | FfiSymbol::RatGt
            | FfiSymbol::RatGe => Ty::boolean(),
            FfiSymbol::RatShow | FfiSymbol::RatToFloat => Ty::text(),
            FfiSymbol::RatFromFloat | FfiSymbol::RatLiteral => Ty::rational(),
            FfiSymbol::IntToRational => Ty::rational(),
            // Text
            FfiSymbol::StringConcat => Ty::text(),
            FfiSymbol::StringLen => Ty::int(),
            FfiSymbol::TextLiteral => Ty::text(),
            FfiSymbol::IntToText | FfiSymbol::BoolToText => Ty::text(),
            FfiSymbol::TextReplaceFirst => Ty::text(),
            // I/O
            FfiSymbol::Print | FfiSymbol::Println => Ty::Unit,
            // Arena
            FfiSymbol::ArenaCreate | FfiSymbol::ArenaAlloc => Ty::int(),
            FfiSymbol::ArenaDestroy => Ty::Unit,
            // Sum
            FfiSymbol::StoreSumResult | FfiSymbol::SumTagInt => Ty::int(),
        }
    }

    /// Constrói FfiSymbol a partir do nome do símbolo C.
    pub fn from_name(name: &str) -> Option<FfiSymbol> {
        let all = [
            FfiSymbol::BiAdd,
            FfiSymbol::BiSub,
            FfiSymbol::BiMul,
            FfiSymbol::BiDiv,
            FfiSymbol::BiEq,
            FfiSymbol::BiNeq,
            FfiSymbol::BiLt,
            FfiSymbol::BiLe,
            FfiSymbol::BiGt,
            FfiSymbol::BiGe,
            FfiSymbol::BiShow,
            FfiSymbol::BiToRational,
            FfiSymbol::TagInt,
            FfiSymbol::Fadd,
            FfiSymbol::Fsub,
            FfiSymbol::Fmul,
            FfiSymbol::Fdiv,
            FfiSymbol::FcmpEq,
            FfiSymbol::FcmpNeq,
            FfiSymbol::FcmpLt,
            FfiSymbol::FcmpLe,
            FfiSymbol::FcmpGt,
            FfiSymbol::FcmpGe,
            FfiSymbol::RatAdd,
            FfiSymbol::RatSub,
            FfiSymbol::RatMul,
            FfiSymbol::RatDiv,
            FfiSymbol::RatEq,
            FfiSymbol::RatNeq,
            FfiSymbol::RatLt,
            FfiSymbol::RatLe,
            FfiSymbol::RatGt,
            FfiSymbol::RatGe,
            FfiSymbol::RatShow,
            FfiSymbol::RatToFloat,
            FfiSymbol::RatFromFloat,
            FfiSymbol::RatLiteral,
            FfiSymbol::IntToRational,
            FfiSymbol::StringConcat,
            FfiSymbol::StringLen,
            FfiSymbol::TextLiteral,
            FfiSymbol::IntToText,
            FfiSymbol::BoolToText,
            FfiSymbol::TextReplaceFirst,
            FfiSymbol::Print,
            FfiSymbol::Println,
            FfiSymbol::ArenaCreate,
            FfiSymbol::ArenaAlloc,
            FfiSymbol::ArenaDestroy,
            FfiSymbol::StoreSumResult,
            FfiSymbol::SumTagInt,
        ];
        all.iter().copied().find(|s| s.symbol_name() == name)
    }
}
