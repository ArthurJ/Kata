//! Testes E2E de codegen — operações Text indexável.
//!
//! Text indexável: t.N (codepoint), len(t), t.[start..end].
//! Text ↔ Bytes roundtrip e codificação UTF-8 estão em bytes_conversions.rs.

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
