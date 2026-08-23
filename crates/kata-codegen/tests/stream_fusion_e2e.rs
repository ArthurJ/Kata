//! Testes E2E de codegen de stream fusion (DoD 60).
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Valida DoD 60: composições de map/filter fundidas em um único loop.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;

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

/// Combina prelude + módulo do usuário.
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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Percorre uma Cons list e retorna todos os elementos (untag SMI).
fn list_to_ints(raw: i64) -> Vec<i64> {
    let mut result = Vec::new();
    let mut ptr = raw;
    while ptr != 0 {
        unsafe {
            let head = *(ptr as *const i64);
            let tail = *((ptr as *const i64).add(1));
            result.push(untag_smi(head));
            ptr = tail;
        }
    }
    result
}

// ═══════════════════════════════════════════════════════════════
// DoD 60: map (+ 10 _) (filter (> _ 5) [1 8 3 9]) → [18 19]
// ═══════════════════════════════════════════════════════════════

/// DoD 60: `map (+ 10 _) (filter (> _ 5) [1 8 3 9])` → [18 19].
/// Stream fusion: map e filter fundidos em um único loop.
#[test]
fn map_sobre_filter_stream_fusion() {
    let src = "map (+ 10 _) (filter (> _ 5) [1 8 3 9])";
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::List(Box::new(Ty::int())),
        "map∘filter deve retornar List(Int)"
    );
    let elems = list_to_ints(raw);
    assert_eq!(
        elems,
        vec![18, 19],
        "map (+10) (filter (>5) [1 8 3 9]) = [18 19]"
    );
}

/// DoD 60 com lambda explícito: `map (lambda x: + x 10) (filter (lambda x: > x 5) [1 8 3 9])`.
#[test]
fn map_sobre_filter_com_lambda_stream_fusion() {
    let src = "map (lambda x: + x 10) (filter (lambda x: > x 5) [1 8 3 9])";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::List(Box::new(Ty::int())));
    let elems = list_to_ints(raw);
    assert_eq!(elems, vec![18, 19]);
}

// ═══════════════════════════════════════════════════════════════
// Fusão de maps: map (+ 1 _) (map (+ 10 _) [1 2 3]) → [12 13 14]
// ═══════════════════════════════════════════════════════════════

/// `map (+ 1 _) (map (+ 10 _) [1 2 3])` → [12 13 14].
/// Dois maps fundidos em um único loop.
#[test]
fn map_sobre_map_stream_fusion() {
    let src = "map (+ 1 _) (map (+ 10 _) [1 2 3])";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::List(Box::new(Ty::int())));
    let elems = list_to_ints(raw);
    assert_eq!(elems, vec![12, 13, 14]);
}

// ═══════════════════════════════════════════════════════════════
// Filter sobre map: filter (> _ 5) (map (+ 10 _) [1 2 3]) → [11 12 13]
// ═══════════════════════════════════════════════════════════════

/// `filter (> _ 5) (map (+ 10 _) [1 2 3])` → [11 12 13].
/// map aplica (+10) → [11 12 13], filter (>5) mantém todos (>5).
#[test]
fn filter_sobre_map_stream_fusion() {
    let src = "filter (> _ 5) (map (+ 10 _) [1 2 3])";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::List(Box::new(Ty::int())));
    let elems = list_to_ints(raw);
    assert_eq!(elems, vec![11, 12, 13]);
}

// ═══════════════════════════════════════════════════════════════
// Três estágios: map (+ 1 _) (filter (> _ 5) (map (+ 10 _) [1 2 3]))
// → filter (>5) em [11 12 13] = [11 12 13], map (+1) = [12 13 14]
// ═══════════════════════════════════════════════════════════════

/// `map (+ 1 _) (filter (> _ 5) (map (+ 10 _) [1 2 3]))` → [12 13 14].
/// Três estágios fundidos: Map, Filter, Map.
#[test]
fn tres_estagios_stream_fusion() {
    let src = "map (+ 1 _) (filter (> _ 5) (map (+ 10 _) [1 2 3]))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::List(Box::new(Ty::int())));
    let elems = list_to_ints(raw);
    assert_eq!(elems, vec![12, 13, 14]);
}

// ═══════════════════════════════════════════════════════════════
// Filter sobre filter: filter (> _ 3) (filter (> _ 1) [1 2 3 4])
// → filter (>1) = [2 3 4], filter (>3) = [4]
// ═══════════════════════════════════════════════════════════════

/// `filter (> _ 3) (filter (> _ 1) [1 2 3 4])` → [4].
/// Dois filters fundidos.
#[test]
fn filter_sobre_filter_stream_fusion() {
    let src = "filter (> _ 3) (filter (> _ 1) [1 2 3 4])";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::List(Box::new(Ty::int())));
    let elems = list_to_ints(raw);
    assert_eq!(elems, vec![4]);
}

// ═══════════════════════════════════════════════════════════════
// map isolado (não-fundido) continua funcionando
// ═══════════════════════════════════════════════════════════════

/// `map (+ 10 _) [1 2 3]` → [11 12 13].
/// Map isolado não deve ser afetado pelo stream fusion pass.
#[test]
fn map_isolado_continua_funcionando() {
    let src = "map (+ 10 _) [1 2 3]";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::List(Box::new(Ty::int())));
    let elems = list_to_ints(raw);
    assert_eq!(elems, vec![11, 12, 13]);
}

/// `filter (> _ 5) [1 8 3 9]` → [8 9].
/// Filter isolado não deve ser afetado.
#[test]
fn filter_isolado_continua_funcionando() {
    let src = "filter (> _ 5) [1 8 3 9]";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::List(Box::new(Ty::int())));
    let elems = list_to_ints(raw);
    assert_eq!(elems, vec![8, 9]);
}
