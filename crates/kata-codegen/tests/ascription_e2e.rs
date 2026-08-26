//! Testes E2E de codegen de ascription-construção.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::StructKey;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;

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

fn infer_src(src: &str) -> Result<kata_inference::TypedModule, kata_diagnostics::MiddleError> {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved)
}

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
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: {
            let mut ir = prelude.interface_registry.clone();
            ir.merge(user.interface_registry.clone());
            ir
        },
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
        },
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

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ═══════════════════════════════════════════════════════════════
// Ascription-construção — DoD 8: (a, b)::Struct promove
// ═══════════════════════════════════════════════════════════════

/// `("João", 30)::Pessoa` produz Ty::Struct("Pessoa").
#[test]
fn ascription_construcao_basica() {
    let src = "data Pessoa (nome::Text idade::Int)\n(\"João\", 30)::Pessoa";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct(StructKey::Plain("Pessoa".into())));
    let _ = raw;
}

/// Ascription-construção com field access — acessa campo da tupla promovida.
#[test]
fn ascription_construcao_field_access() {
    let src = "data Pessoa (nome::Text idade::Int)\nconstant p := (\"João\", 30)::Pessoa\np.idade";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 30);
}

/// Ascription-construção com 3 campos.
#[test]
fn ascription_construcao_tres_campos() {
    let src = "data Trio (a::Int b::Int c::Int)\nconstant t := (1, 2, 3)::Trio\nt.b";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 2);
}

/// Shape mismatch — número errado de elementos.
#[test]
fn ascription_construcao_shape_mismatch() {
    let src = "data Pessoa (nome::Text idade::Int)\n(\"João\", 30, 42)::Pessoa";
    let result = infer_src(src);
    assert!(result.is_err(), "shape mismatch deve falhar");
}

/// Shape mismatch — tipos incompatíveis.
#[test]
fn ascription_construcao_tipo_mismatch() {
    let src = "data Pessoa (nome::Text idade::Int)\n(42, \"João\")::Pessoa";
    let result = infer_src(src);
    assert!(result.is_err(), "tipo mismatch deve falhar");
}
