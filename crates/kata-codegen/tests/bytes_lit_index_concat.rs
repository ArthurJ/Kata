//! Testes E2E de codegen de Bytes e Byte (PRD-bytes Fase 6).
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Cobertura: BytesLit, indexação, concatenação, len, slice, show, eq,
//! bitwise (and/or/xor/not), conversões (int/byte/bytes),
//! Text indexável (at/len/slice), Text ↔ Bytes.

#![allow(non_snake_case)]

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
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
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
        type_graph: {
            let mut tg = prelude.type_graph.clone();
            tg.merge(&user.type_graph);
            tg
        },
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
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
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

/// `b'Hello'` (aspas simples) cria mesmo blob que `b"Hello"`.
#[test]
fn bytes_lit_single_quotes_retorna_bytes() {
    let src = "b'Hello'";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Bytes, "b'Hello' deve ter tipo Bytes");
    assert_ne!(raw, 0, "blob não deve ser null");
}

/// `b'Hello'` e `b"Hello"` produzem blobs com mesmo len e conteúdo.
#[test]
fn bytes_lit_single_and_double_quotes_equivalent() {
    let (raw_single, ty_single) = eval_src("len(b'Hello')");
    let (raw_double, ty_double) = eval_src("len(b\"Hello\")");
    assert_eq!(ty_single, ty_double);
    assert_eq!(untag_smi(raw_single), untag_smi(raw_double));
    assert_eq!(untag_smi(raw_single), 5);
}

/// `b'\x00\xFF'` (aspas simples + hex escape) produz 2 bytes.
#[test]
fn bytes_lit_single_quotes_hex_escape() {
    let src = "len(b'\\x00\\xFF')";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 2, "b'\\x00\\xFF' = 2 bytes");
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
