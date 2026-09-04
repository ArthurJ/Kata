//! Testes E2E: enum da stdlib como type parameter de genérico (A10).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Valida que enums definidos na stdlib (ex: `Encoding`) são corretamente
//! resolvidos como `Sum("Encoding")` quando usados em anotações de tipo do
//! usuário (ex: `foo :: Result::(Int, Encoding) => Text`).
//!
//! Antes do fix, `resolve_type_expr` não encontrava `Encoding` no TypeEnv
//! do módulo do usuário (o binding da stdlib só ficava disponível após
//! `merge_two`), e caía para `Struct(Plain("Encoding"))` em vez de
//! `Sum("Encoding")`. O dispatch então falhava com `type.no_overload`
//! porque `Struct(Plain("Encoding"))` != `Sum("Encoding")`.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{
    DirectiveRegistry, ResolvedModule, load_stdlib_for_tests, resolve_with_prelude,
};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve_with_prelude(
        &module,
        "__local__",
        DirectiveRegistry::new(),
        &prelude.interface_registry,
        &prelude.directive_registry,
        Some(&prelude.type_graph),
        Some(&prelude.type_env),
    )
    .expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Combina prelude + módulo do usuário (replica do driver).
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
        directive_registry: DirectiveRegistry::new(),
    }
}

/// A10 stdlib: `Encoding` (da stdlib) como type param de `Result`.
/// `foo (Ok 5)` — a variante `Ok` não menciona `E`, então o hint do call site
/// propaga `Result::(Int, Encoding)` e o `Var("E")` sobrevive até o dispatch.
/// `foo (Err Encoding::Utf8)` — a variante `Err` menciona `E`, infere
/// `Sum("Encoding")` do argumento.
#[test]
fn enum_stdlib_como_type_param_de_generico() {
    let src = r#"foo :: Result::(Int, Encoding) => Text
lambda m:
    match m
        Ok n: "ok"
        Err e: "erro"

action main
    echo!(foo (Ok 5))
    echo!(foo (Err Encoding::Utf8))
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    // main!() retorna Unit (SMI 0). As chamadas echo! imprimem "ok" e "erro".
    let _ = raw;
}

/// A10 stdlib: apenas `Ok` (variante que não menciona o type param do enum).
/// O hint do call site propaga `Result::(Int, Encoding)` e o default NÃO roda
/// (estratégia C: defaults só quando `expected_ty.is_none()`).
#[test]
fn enum_stdlib_ok_variante_sem_type_param() {
    let src = r#"foo :: Result::(Int, Encoding) => Text
lambda m:
    match m
        Ok n: "ok"
        Err e: "erro"

action main
    echo!(foo (Ok 5))
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    let _ = raw;
}

/// A10 stdlib: apenas `Err` (variante que menciona o type param do enum).
/// `Err Encoding::Utf8` infere `E = Sum("Encoding")` do argumento.
#[test]
fn enum_stdlib_err_variante_com_type_param() {
    let src = r#"foo :: Result::(Int, Encoding) => Text
lambda m:
    match m
        Ok n: "ok"
        Err e: "erro"

action main
    echo!(foo (Err Encoding::Utf8))
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    let _ = raw;
}