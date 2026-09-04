//! Testes E2E de codegen — operações bitwise em Byte e Bytes.
//!
//! Bitwise Byte: and, or, xor, not (escalar).
//! Bitwise Bytes: and, or, xor, not (elemento-a-elemento).
//! Bitwise Bytes broadcast: tamanhos diferentes.

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
// Bitwise Bytes — broadcast (tamanhos diferentes)
// ═══════════════════════════════════════════════════════════════════

/// `and b"\xFF\xF0" b"\xAA"` — broadcast: resultado tem 2 bytes.
/// byte 0: 0xFF AND 0xAA = 0xAA. byte 1: 0xF0 AND 0 (pad) = 0x00.
#[test]
fn bytes_and_broadcast_tamanho_do_maior() {
    let src = "len(and b\"\\xFF\\xF0\" b\"\\xAA\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(
        untag_smi(raw),
        2,
        "broadcast: resultado tem tamanho do maior"
    );
}

/// `and b"\xFF\xF0" b"\xAA"` byte 1 = 0x00 (0xF0 AND 0x00 pad).
#[test]
fn bytes_and_broadcast_byte_extra_eh_zero() {
    let src = "(and b\"\\xFF\\xF0\" b\"\\xAA\").1 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x00, "0xF0 AND 0 (pad) = 0x00");
}

/// `or b"\xF0\x0F" b"\x0F"` — broadcast: byte 1 preserva (0x0F OR 0 = 0x0F).
#[test]
fn bytes_or_broadcast_preserva_byte_extra() {
    let src = "(or b\"\\xF0\\x0F\" b\"\\x0F\").1 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x0F, "0x0F OR 0 (pad) = 0x0F");
}

/// `xor b"\xFF\x42" b"\x0F"` — broadcast: byte 1 preserva (0x42 XOR 0 = 0x42).
#[test]
fn bytes_xor_broadcast_preserva_byte_extra() {
    let src = "(xor b\"\\xFF\\x42\" b\"\\x0F\").1 | byte(0)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Byte);
    assert_eq!(untag_smi(raw), 0x42, "0x42 XOR 0 (pad) = 0x42");
}
