//! Testes E2E de `show` universal: Tuple, Array, Set, Dict, List, Unit, repr.
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Valida que `show` produce Text correto para todos os tipos compostos.
//! Valida que `repr` cita Text e delega para show nos demais.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
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

/// Lê uma C string (Text) do ponteiro retornado pelo JIT.
fn read_text(raw: i64) -> String {
    unsafe {
        let cstr = std::ffi::CStr::from_ptr(raw as *const std::os::raw::c_char);
        cstr.to_string_lossy().into_owned()
    }
}

/// Avalia `show <expr>` e retorna o Text resultante.
fn show_eval(src: &str) -> String {
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text(), "show deve retornar Text");
    read_text(raw)
}

// ═══════════════════════════════════════════════════════════════
// Tuple
// ═══════════════════════════════════════════════════════════════

#[test]
fn show_tuple_two_elements() {
    let result = show_eval("show (1, \"hello\")");
    assert_eq!(result, "(1, \"hello\")");
}

#[test]
fn show_tuple_three_ints() {
    let result = show_eval("show (1, 2, 3)");
    assert_eq!(result, "(1, 2, 3)");
}

#[test]
fn show_tuple_single_element() {
    let result = show_eval("show (42,)");
    assert_eq!(result, "(42)");
}

// ═══════════════════════════════════════════════════════════════
// Unit
// ═══════════════════════════════════════════════════════════════

#[test]
fn show_unit() {
    let result = show_eval("show ()");
    assert_eq!(result, "()");
}

// ═══════════════════════════════════════════════════════════════
// Array
// ═══════════════════════════════════════════════════════════════

#[test]
fn show_array_ints() {
    let result = show_eval("show {1 2 3}");
    assert_eq!(result, "{1, 2, 3}");
}

#[test]
fn show_array_empty() {
    let result = show_eval("show {}");
    assert_eq!(result, "{}");
}

#[test]
fn show_array_text() {
    let result = show_eval("show {\"a\" \"b\"}");
    assert_eq!(result, "{\"a\", \"b\"}");
}

// ═══════════════════════════════════════════════════════════════
// Set
// ═══════════════════════════════════════════════════════════════

#[test]
fn show_set_ints() {
    let result = show_eval("show {|1 2 3|}");
    // Ordem reversa: Cons prepend (mais novo primeiro)
    assert_eq!(result, "{|3, 2, 1|}");
}

#[test]
fn show_set_single() {
    let result = show_eval("show {|1|}");
    assert_eq!(result, "{|1|}");
}

#[test]
fn show_set_text() {
    let result = show_eval("show {|\"a\" \"b\" \"c\"|}");
    // Ordem reversa + Text citado via repr_expr
    assert_eq!(result, "{|\"c\", \"b\", \"a\"|}");
}

// ═══════════════════════════════════════════════════════════════
// Dict
// ═══════════════════════════════════════════════════════════════

#[test]
fn show_dict_text_int() {
    let result = show_eval("show {\"a\": 1 \"b\": 2}");
    // Ordem reversa: b vem antes de a
    assert_eq!(result, "{\"b\": 2, \"a\": 1}");
}

#[test]
fn show_dict_text_text() {
    let result = show_eval("show {\"nome\": \"Ana\"}");
    assert_eq!(result, "{\"nome\": \"Ana\"}");
}

#[test]
fn show_dict_three_entries() {
    let result = show_eval("show {\"a\": 1 \"b\": 2 \"c\": 3}");
    // Ordem reversa: c, b, a
    assert_eq!(result, "{\"c\": 3, \"b\": 2, \"a\": 1}");
}

// ═══════════════════════════════════════════════════════════════
// List
// ═══════════════════════════════════════════════════════════════

#[test]
fn show_list_ints() {
    let result = show_eval("show [1 2 3]");
    assert_eq!(result, "[1, 2, 3]");
}

#[test]
fn show_list_empty() {
    let result = show_eval("show []");
    assert_eq!(result, "[]");
}

#[test]
fn show_list_text() {
    let result = show_eval("show [\"a\" \"b\"]");
    assert_eq!(result, "[\"a\", \"b\"]");
}

// ═══════════════════════════════════════════════════════════════
// Primitivos (show direto)
// ═══════════════════════════════════════════════════════════════

#[test]
fn show_int() {
    let result = show_eval("show 42");
    assert_eq!(result, "42");
}

#[test]
fn show_text_identity() {
    let result = show_eval("show \"hello\"");
    assert_eq!(result, "hello");
}

// ═══════════════════════════════════════════════════════════════
// repr standalone
// ═══════════════════════════════════════════════════════════════

#[test]
fn repr_text_quotes() {
    let (raw, ty) = eval_src("repr \"hello\"");
    assert_eq!(ty, Ty::text());
    assert_eq!(read_text(raw), "\"hello\"");
}

#[test]
fn repr_int_delegates_to_show() {
    let (raw, ty) = eval_src("repr 42");
    assert_eq!(ty, Ty::text());
    assert_eq!(read_text(raw), "42");
}

// ═══════════════════════════════════════════════════════════════
// Aninhamentos
// ═══════════════════════════════════════════════════════════════

#[test]
fn show_list_of_tuples() {
    let result = show_eval("show [(1, \"a\") (2, \"b\")]");
    assert_eq!(result, "[(1, \"a\"), (2, \"b\")]");
}

#[test]
fn show_array_of_tuples() {
    let result = show_eval("show {(1, \"a\") (2, \"b\")}");
    assert_eq!(result, "{(1, \"a\"), (2, \"b\")}");
}

#[test]
fn show_list_of_lists() {
    let result = show_eval("show [[1 2] [3 4]]");
    assert_eq!(result, "[[1, 2], [3, 4]]");
}
