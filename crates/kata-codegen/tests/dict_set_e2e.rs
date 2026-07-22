//! Testes E2E de codegen de Dict e Set: literal, lookup, contains, len, ops.
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Valida Dict/Set HAMT operations: literal, len, at, in, insert, union, intersection, difference.

use kata_codegen::jit_eval;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
#[allow(dead_code)]
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Lê uma C string (Text) do ponteiro retornado pelo JIT.
#[allow(dead_code)]
fn read_text(raw: i64) -> String {
    unsafe {
        let cstr = std::ffi::CStr::from_ptr(raw as *const std::os::raw::c_char);
        cstr.to_string_lossy().into_owned()
    }
}

// ═══════════════════════════════════════════════════════════════
// Dict: literal produces Dict(Text, Int)
// ═══════════════════════════════════════════════════════════════

#[test]
fn dict_literal_produces_dict_type() {
    let src = "{\"a\": 1 \"b\": 2}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Dict(Box::new(Ty::Prim(PrimTy::Text)), Box::new(Ty::int())));
    assert_ne!(raw, 0, "dict should be non-null pointer");
}

// ═══════════════════════════════════════════════════════════════
// Dict: len returns 2
// ═══════════════════════════════════════════════════════════════

#[test]
fn dict_len_returns_2() {
    let src = "len {\"a\": 1 \"b\": 2}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 2);
}

// ═══════════════════════════════════════════════════════════════
// Dict: at with ? unwraps Result, returns value for found key
// ═══════════════════════════════════════════════════════════════

#[test]
fn dict_at_found() {
    // `?` only works inside Action — use action block, entry via action call
    let src = "action extrai => Int\n    at {\"a\": 1 \"b\": 2} \"a\" ?\nextrai!()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 1);
}

// ═══════════════════════════════════════════════════════════════
// Dict: contains (in) returns true for existing key
// ═══════════════════════════════════════════════════════════════

#[test]
fn dict_contains_true() {
    let src = "\"a\" in {\"a\": 1 \"b\": 2}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::boolean());
    assert_eq!(raw, 1, "should be true");
}

// ═══════════════════════════════════════════════════════════════
// Dict: contains (in) returns false for missing key
// ═══════════════════════════════════════════════════════════════

#[test]
fn dict_contains_false() {
    let src = "\"c\" in {\"a\": 1 \"b\": 2}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::boolean());
    assert_eq!(raw, 0, "should be false");
}

// ═══════════════════════════════════════════════════════════════
// Set: literal produces Set(Int)
// ═══════════════════════════════════════════════════════════════

#[test]
fn set_literal_produces_set_type() {
    let src = "{|1 2 3|}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Set(Box::new(Ty::int())));
    assert_ne!(raw, 0);
}

// ═══════════════════════════════════════════════════════════════
// Set: len returns 3
// ═══════════════════════════════════════════════════════════════

#[test]
fn set_len_returns_3() {
    let src = "len {|1 2 3|}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 3);
}

// ═══════════════════════════════════════════════════════════════
// Set: contains (in) returns true for existing element
// ═══════════════════════════════════════════════════════════════

#[test]
fn set_contains_true() {
    let src = "3 in {|1 2 3|}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::boolean());
    assert_eq!(raw, 1);
}

// ═══════════════════════════════════════════════════════════════
// Set: contains (in) returns false for missing element
// ═══════════════════════════════════════════════════════════════

#[test]
fn set_contains_false() {
    let src = "5 in {|1 2 3|}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::boolean());
    assert_eq!(raw, 0);
}

// ═══════════════════════════════════════════════════════════════
// Set operations: union, intersection, difference
// ═══════════════════════════════════════════════════════════════

#[test]
fn set_union_size() {
    let src = "len (union {|1 2 3|} {|3 4 5|})";
    let (raw, _) = eval_src(src);
    assert_eq!(untag_smi(raw), 5, "union of {{1,2,3}} and {{3,4,5}} should have 5 elements");
}

#[test]
fn set_intersection_size() {
    let src = "len (intersection {|1 2 3|} {|3 4 5|})";
    let (raw, _) = eval_src(src);
    assert_eq!(untag_smi(raw), 1, "intersection of {{1,2,3}} and {{3,4,5}} should have 1 element");
}

#[test]
fn set_difference_size() {
    let src = "len (difference {|1 2 3|} {|3 4 5|})";
    let (raw, _) = eval_src(src);
    assert_eq!(untag_smi(raw), 2, "difference of {{1,2,3}} and {{3,4,5}} should have 2 elements");
}

// ═══════════════════════════════════════════════════════════════
// Dict insert: adds new key, replaces existing value
// ═══════════════════════════════════════════════════════════════

#[test]
fn dict_insert_adds_key() {
    let src = "len (insert {\"a\": 1} \"b\" 2)";
    let (raw, _) = eval_src(src);
    assert_eq!(untag_smi(raw), 2);
}

#[test]
fn dict_insert_replaces_value() {
    let src = "action extrai2 => Int\n    at (insert {\"a\": 1} \"a\" 99) \"a\" ?\nextrai2!()";
    let (raw, _) = eval_src(src);
    assert_eq!(untag_smi(raw), 99);
}