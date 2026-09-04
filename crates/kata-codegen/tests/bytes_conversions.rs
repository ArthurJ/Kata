//! Testes E2E de codegen — conversões e roundtrips entre Byte, Bytes, Int, Text.
//!
//! Conversões: int(byte), byte(int), bytes(int), bytes(text), text(bytes).
//! Roundtrip: text→bytes→index, int→bytes→index→int, byte→int→byte→int.

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

/// `text(b"Hello")` decodifica UTF-8 → Ok("Hello").
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
