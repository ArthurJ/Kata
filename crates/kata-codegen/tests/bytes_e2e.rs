//! Testes E2E de codegen de Bytes e Byte (PRD-bytes Fase 6).
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Cobertura: BytesLit, indexação, concatenação, len, slice, show, eq,
//! bitwise (and/or/xor/not), conversões (int/byte/bytes),
//! Text indexável (at/len/slice), Text ↔ Bytes.

use kata_codegen::jit_eval;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

/// Combina prelude + módulo do usuário (replica do driver com merge completo).
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);

    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);

    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);

    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);

    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);

    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);

    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);

    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
        functions: {
            let mut fns = prelude.functions;
            let user_fn_names: std::collections::HashSet<&str> =
                user.functions.iter().map(|f| f.name.as_str()).collect();
            fns.retain(|f| !user_fn_names.contains(f.name.as_str()));
            fns.extend(user.functions);
            fns
        },
        actions: {
            let mut acts = prelude.actions;
            let user_action_names: std::collections::HashSet<&str> =
                user.actions.iter().map(|a| a.name.as_str()).collect();
            acts.retain(|a| !user_action_names.contains(a.name.as_str()));
            acts.extend(user.actions);
            acts
        },
    }
}

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default()).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Desfaz SMI tagging: (raw >> 1).
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ═══════════════════════════════════════════════════════════════════
// BytesLit — criação e tipo
// ═══════════════════════════════════════════════════════════════════

/// `b"Hello"` cria um blob de 5 bytes. O valor é ponteiro na arena.
#[test]
fn bytes_lit_retorna_bytes() {
    let src = "b\"Hello\"";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Bytes, "b\"Hello\" deve ter tipo Bytes");
    // Bytes é ponteiro na arena — não é SMI. Apenas verifica que não é 0 (null).
    assert_ne!(raw, 0, "blob não deve ser null");
}

/// `b""` cria blob vazio de 0 bytes.
#[test]
fn bytes_lit_vazio_retorna_bytes() {
    let src = "b\"\"";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Bytes);
    assert_ne!(raw, 0, "blob vazio não deve ser null (tem header)");
}

// ═══════════════════════════════════════════════════════════════════
// Indexação — b.N retorna Result::(Byte, Text), | extrai Byte
// ═══════════════════════════════════════════════════════════════════

/// `b"Hello".0 | byte(0)` retorna 0x48 = 72 = 'H'.
#[test]
fn bytes_index_primeiro_byte_retorna_H() {
    let src = "b\"Hello\".0 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte, "b.0 | byte(0) deve retornar Byte");
    assert_eq!(
        untag_smi(raw),
        0x48,
        "primeiro byte de \"Hello\" = 0x48 = 'H'"
    );
}

/// `b"Hello".4 | byte(0)` retorna 0x6F = 111 = 'o'.
#[test]
fn bytes_index_ultimo_byte_retorna_o() {
    let src = "b\"Hello\".4 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(
        untag_smi(raw),
        0x6F,
        "quinto byte de \"Hello\" = 0x6F = 'o'"
    );
}

/// `b"ABC".(-1) | byte(0)` retorna 0x43 = 67 = 'C' (índice negativo).
#[test]
fn bytes_index_negativo_retorna_ultimo() {
    let src = "b\"ABC\".(-1) | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x43, "último byte de \"ABC\" = 0x43 = 'C'");
}

/// `b"ABC".(-3) | byte(0)` retorna 0x41 = 65 = 'A' (índice negativo no início).
#[test]
fn bytes_index_negativo_retorna_primeiro() {
    let src = "b\"ABC\".(-3) | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(
        untag_smi(raw),
        0x41,
        "primeiro byte de \"ABC\" = 0x41 = 'A'"
    );
}

/// `b"Hi".5 | byte(99)` retorna 99 (out of bounds → fallback).
#[test]
fn bytes_index_out_of_bounds_retorna_fallback() {
    let src = "b\"Hi\".5 | byte(99)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(
        untag_smi(raw),
        99,
        "out of bounds deve retornar fallback byte(99)"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Concatenação — + b1 b2
// ═══════════════════════════════════════════════════════════════════

/// `+ b"AB" b"CD"` produz 4 bytes. Verifica via len.
#[test]
fn bytes_concat_produz_4_bytes() {
    let src = "len (+ b\"AB\" b\"CD\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 4, "concat de \"AB\" + \"CD\" = 4 bytes");
}

/// Concatenação preserva conteúdo: primeiro byte de (b"X" + b"Y") = 'X'.
#[test]
fn bytes_concat_preserva_conteudo() {
    let src = "(+ b\"X\" b\"Y\").0 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x58, "primeiro byte de \"XY\" = 0x58 = 'X'");
}

/// Concatenação: segundo byte de (b"X" + b"Y") = 'Y'.
#[test]
fn bytes_concat_segundo_byte() {
    let src = "(+ b\"X\" b\"Y\").1 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x59, "segundo byte de \"XY\" = 0x59 = 'Y'");
}

// ═══════════════════════════════════════════════════════════════════
// len — número de bytes
// ═══════════════════════════════════════════════════════════════════

/// `len(b"Hello")` retorna 5.
#[test]
fn bytes_len_retorna_5() {
    let src = "len(b\"Hello\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 5, "len(b\"Hello\") = 5");
}

/// `len(b"")` retorna 0.
#[test]
fn bytes_len_vazio_retorna_0() {
    let src = "len(b\"\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 0, "len(b\"\") = 0");
}

// ═══════════════════════════════════════════════════════════════════
// Slice — b.[start..end]
// ═══════════════════════════════════════════════════════════════════

/// `b"Hello".[1..3]` produz 2 bytes ("el").
#[test]
fn bytes_slice_1_to_3_retorna_2_bytes() {
    let src = "len(b\"Hello\".[1..3])";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 2, "slice [1..3] de \"Hello\" = 2 bytes");
}

/// `b"ABCDEF".[2..=4]` produz 3 bytes (inclusive).
#[test]
fn bytes_slice_inclusivo_2_to_4_retorna_3_bytes() {
    let src = "len(b\"ABCDEF\".[2..=4])";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 3, "slice [2..=4] de \"ABCDEF\" = 3 bytes");
}

/// Slice preserva conteúdo: primeiro byte de b"Hello".[1..3] = 'e'.
#[test]
fn bytes_slice_preserva_conteudo() {
    let src = "b\"Hello\".[1..3].0 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(
        untag_smi(raw),
        0x65,
        "primeiro byte do slice [1..3] = 0x65 = 'e'"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Show — representação hex
// ═══════════════════════════════════════════════════════════════════

/// `show(b"Hi")` retorna Text (hex). Tipo é Text (ponteiro na arena).
#[test]
fn bytes_show_retorna_text() {
    let src = "show(b\"Hi\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text(), "show deve retornar Text");
    // Text é ponteiro — não podemos verificar conteúdo sem FFI de leitura.
    let _ = raw;
}

// ═══════════════════════════════════════════════════════════════════
// EQ — comparação byte-a-byte
// ═══════════════════════════════════════════════════════════════════

/// `= b"Hello" b"Hello"` retorna True.
#[test]
fn bytes_eq_iguais_retorna_true() {
    let src = "= b\"Hello\" b\"Hello\"";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::boolean());
    // Boolean é inline: True = 1, False = 0 (não SMI-tagged).
    assert_eq!(raw, 1, "True = 1");
}

/// `= b"Hello" b"World"` retorna False.
#[test]
fn bytes_eq_diferentes_retorna_false() {
    let src = "= b\"Hello\" b\"World\"";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::boolean());
    assert_eq!(raw, 0, "False = 0");
}

// ═══════════════════════════════════════════════════════════════════
// Bitwise Byte — and, or, xor, not
// ═══════════════════════════════════════════════════════════════════

/// `and (byte 0xF0) (byte 0x0C)` = 0x00 (AND bit-a-bit).
#[test]
fn byte_and_retorna_0x00() {
    let src = "and (byte 240) (byte 12)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x00, "0xF0 AND 0x0C = 0x00");
}

/// `or (byte 0xF0) (byte 0x0C)` = 0xFC (OR bit-a-bit).
#[test]
fn byte_or_retorna_0xFC() {
    let src = "or (byte 240) (byte 12)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0xFC, "0xF0 OR 0x0C = 0xFC");
}

/// `xor (byte 0xFF) (byte 0x0F)` = 0xF0 (XOR bit-a-bit).
#[test]
fn byte_xor_retorna_0xF0() {
    let src = "xor (byte 255) (byte 15)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0xF0, "0xFF XOR 0x0F = 0xF0");
}

/// `not (byte 0x00)` = 0xFF (NOT inverte todos os bits).
#[test]
fn byte_not_retorna_0xFF() {
    let src = "not (byte 0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0xFF, "NOT 0x00 = 0xFF");
}

/// `not (byte 0xFF)` = 0x00.
#[test]
fn byte_not_retorna_0x00() {
    let src = "not (byte 255)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x00, "NOT 0xFF = 0x00");
}

// ═══════════════════════════════════════════════════════════════════
// Bitwise Bytes — and, or, xor, not (elemento-a-elemento)
// ═══════════════════════════════════════════════════════════════════

/// `and b"\\xFF\\xFF" b"\\x0F\\x0F"` = [0x0F 0x0F]. Verifica via len.
#[test]
fn bytes_and_produz_2_bytes() {
    let src = "len(and b\"\\xFF\\xFF\" b\"\\x0F\\x0F\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 2, "AND de 2 bytes = 2 bytes");
}

/// `xor b"\\xFF\\xFF" b"\\x0F\\x0F"` = [0xF0 0xF0]. Primeiro byte = 0xF0.
#[test]
fn bytes_xor_preserva_conteudo() {
    let src = "(xor b\"\\xFF\\xFF\" b\"\\x0F\\x0F\").0 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0xF0, "0xFF XOR 0x0F = 0xF0");
}

/// `not b"\\x00\\xFF"` = [0xFF 0x00]. Primeiro byte = 0xFF.
#[test]
fn bytes_not_preserva_conteudo() {
    let src = "(not b\"\\x00\\xFF\").0 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0xFF, "NOT 0x00 = 0xFF");
}

// ═══════════════════════════════════════════════════════════════════
// Conversões — int(byte), byte(int), bytes(int)
// ═══════════════════════════════════════════════════════════════════

/// `int(byte(72))` = 72 (Byte → Int).
#[test]
fn int_de_byte_retorna_72() {
    let src = "int(byte(72))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 72, "int(byte(72)) = 72");
}

/// `byte(256)` trunca para 0 (mod 256).
#[test]
fn byte_de_256_trunca_para_0() {
    let src = "int(byte(256))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 0, "byte(256) = byte(0) = 0 (mod 256)");
}

/// `bytes(42)` produz 4 bytes (little-endian). len = 4.
#[test]
fn bytes_de_int_retorna_4_bytes() {
    let src = "len(bytes(42))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 4, "bytes(42) = 4 bytes (little-endian)");
}

/// `(bytes 42)` primeiro byte = 0x2A (42 em little-endian = [2A 00 00 00]).
#[test]
fn bytes_de_int_primeiro_byte_2A() {
    let src = "(bytes 42).0 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(
        untag_smi(raw),
        0x2A,
        "42 little-endian primeiro byte = 0x2A"
    );
}

/// `(bytes 42)` segundo byte = 0x00.
#[test]
fn bytes_de_int_segundo_byte_00() {
    let src = "(bytes 42).1 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x00, "42 little-endian segundo byte = 0x00");
}

// ═══════════════════════════════════════════════════════════════════
// Text ↔ Bytes — codificação UTF-8
// ═══════════════════════════════════════════════════════════════════

/// `bytes("Hello")` codifica UTF-8 → 5 bytes.
#[test]
fn bytes_de_text_retorna_5_bytes() {
    let src = "len(bytes(\"Hello\"))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 5, "bytes(\"Hello\") = 5 bytes UTF-8");
}

/// `(bytes "Hello")` primeiro byte = 0x48 = 'H'.
#[test]
fn bytes_de_text_primeiro_byte_H() {
    let src = "(bytes \"Hello\").0 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(
        untag_smi(raw),
        0x48,
        "primeiro byte de \"Hello\" UTF-8 = 0x48"
    );
}

/// `bytes("café")` codifica UTF-8 → 5 bytes (é = 2 bytes).
#[test]
fn bytes_de_text_com_acento_retorna_5_bytes() {
    let src = "len(bytes(\"café\"))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(
        untag_smi(raw),
        5,
        "\"café\" em UTF-8 = 5 bytes (é = 0xC3 0xA9)"
    );
}

/// `text(b"Hello")` decodifica UTF-8 → Result::Ok("Hello").
/// Text é ponteiro — não podemos verificar conteúdo, apenas que não panica.
#[test]
fn text_de_bytes_retorna_result_ok() {
    let src = "text(b\"Hello\")";
    let (raw, ty) = eval_src(src);
    // text(Bytes) retorna Result::(Text, Text) — é um Generic (Sum instanciado).
    assert!(
        matches!(ty, Ty::Generic(ref name, _) if name == "Result"),
        "text(bytes) deve retornar Result, got {ty}"
    );
    let _ = raw;
}

// ═══════════════════════════════════════════════════════════════════
// Text indexável — t.N, len(t), t.[start..end]
// ═══════════════════════════════════════════════════════════════════

/// `"Hello".0 | "?"` retorna "H" (primeiro codepoint).
/// Text é ponteiro — não podemos verificar conteúdo, apenas tipo.
#[test]
fn text_index_primeiro_codepoint_retorna_text() {
    let src = "\"Hello\".0 | \"?\"";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text(), "t.0 | \"?\" deve retornar Text");
    let _ = raw;
}

/// `len("Hello")` retorna 5 (codepoints).
#[test]
fn text_len_retorna_5_codepoints() {
    let src = "len(\"Hello\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 5, "len(\"Hello\") = 5 codepoints");
}

/// `len("café")` retorna 4 (codepoints, não bytes).
#[test]
fn text_len_com_acento_retorna_4_codepoints() {
    let src = "len(\"café\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(
        untag_smi(raw),
        4,
        "len(\"café\") = 4 codepoints (não 5 bytes)"
    );
}

/// `"Hello".(-1) | "?"` retorna "o" (último codepoint).
#[test]
fn text_index_negativo_retorna_ultimo_codepoint() {
    let src = "\"Hello\".(-1) | \"?\"";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `"Hello".[1..3]` retorna "el" (2 codepoints).
/// Text é ponteiro — não podemos verificar conteúdo, apenas tipo.
#[test]
fn text_slice_retorna_text() {
    let src = "\"Hello\".[1..3]";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text(), "t.[1..3] deve retornar Text");
    let _ = raw;
}

/// `len("Hello".[0..3])` retorna 3 (codepoints no slice).
#[test]
fn text_slice_len_3_codepoints() {
    let src = "len(\"Hello\".[0..3])";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(
        untag_smi(raw),
        3,
        "slice [0..3] de \"Hello\" = 3 codepoints"
    );
}

/// `len("Hello".[2..=4])` retorna 3 (inclusive).
#[test]
fn text_slice_inclusivo_len_3_codepoints() {
    let src = "len(\"Hello\".[2..=4])";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(
        untag_smi(raw),
        3,
        "slice [2..=4] de \"Hello\" = 3 codepoints"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Roundtrip Text ↔ Bytes
// ═══════════════════════════════════════════════════════════════════

/// `bytes("Hi")` depois primeiro byte = 'H' = 0x48.
/// Roundtrip implícito: text → bytes → indexação.
#[test]
fn roundtrip_text_para_bytes_preserva_conteudo() {
    let src = "(bytes \"Hi\").1 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(
        untag_smi(raw),
        0x69,
        "segundo byte de \"Hi\" UTF-8 = 0x69 = 'i'"
    );
}

/// Roundtrip: `bytes(42)` → `len` → 4 bytes → indexação → `int` → 42.
/// Verifica que bytes(int) produz little-endian correto.
#[test]
fn roundtrip_int_para_bytes_little_endian() {
    // 42 = [2A 00 00 00] em little-endian
    // int(first_byte) = 0x2A = 42
    let src = "int((bytes 42).0 | byte(0))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 42, "primeiro byte de bytes(42) = 0x2A = 42");
}

/// Roundtrip completo: byte → int → byte preserva valor.
#[test]
fn roundtrip_byte_int_preserva_valor() {
    let src = "int(byte(int(byte(123))))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 123, "byte→int→byte→int preserva 123");
}
