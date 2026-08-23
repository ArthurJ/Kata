//! Testes de default methods em interfaces.
//!
//! Uma interface pode ter `default_body` (cláusulas `lambda` após a
//! assinatura). Tipos que implementam a interface sem definir o método
//! usam o default. Tipos que definem o método sobrescrevem (shadow).
//!
//! Cenários:
//! 1. Interface com default, tipo sem shadow → usa default
//! 2. Interface com default, tipo com shadow → usa próprio
//! 3. Interface sem default (assinatura obrigatória) → tipo deve definir

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{load_stdlib_for_tests, resolve};

fn merge_resolved(
    prelude: kata_resolution::ResolvedModule,
    user: kata_resolution::ResolvedModule,
) -> kata_resolution::ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);
    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);
    kata_resolution::ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
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

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("infer deve succeed")
}

/// Interface com default method. Tipo sem shadow usa o default.
#[test]
fn default_method_used_when_no_shadow() {
    let src = r#"
interface GREETER
    greet :: Self => Text
    lambda x:
        "hello"

Int implements GREETER

greet 42
"#;
    let typed = infer_src(src);
    // greet 42 com Int deve retornar Text (usa default method).
    assert_eq!(typed.entry.node.ty, Ty::Prim(kata_core::ty::PrimTy::Text));
}

/// Interface com default method. Tipo com shadow usa o próprio.
#[test]
fn default_method_shadowed_by_impl() {
    let src = r#"
interface GREETER
    greet :: Self => Text
    lambda x:
        "hello"

Int implements GREETER
    @ffi("kata_rt_bi_show")
    greet :: Int => Text

greet 42
"#;
    let typed = infer_src(src);
    // greet 42 com Int deve retornar Text (usa impl, não default).
    assert_eq!(typed.entry.node.ty, Ty::Prim(kata_core::ty::PrimTy::Text));
}

/// Interface com default que retorna Self. O typeck deve substituir
/// Self pelo tipo concreto.
#[test]
fn default_method_self_in_return() {
    let src = r#"
interface STEPPER
    next :: Self => Self
    lambda x:
        x

Int implements STEPPER
    @ffi("kata_rt_bi_add")
    next :: Int => Int

next 42
"#;
    let typed = infer_src(src);
    // next 42 com Int deve retornar Int (shadow do impl).
    assert_eq!(typed.entry.node.ty, Ty::int());
}
