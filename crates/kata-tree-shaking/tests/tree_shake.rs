//! Testes E2E de tree shaking — removem código morto do TypedModule.
//!
//! Pipeline completo até tree_shake: lex → parse → resolve → infer →
//! monomorphize → optimize → tree_shake. Verifica que funções/actions
//! não alcançadas e `@test` specs são removidos, e que funções alcançadas
//! transitivamente são mantidas.

use kata_core::InterfaceRegistry;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

/// Merge prelude + user resolved modules — mesmo helper dos testes E2E.
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
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: InterfaceRegistry::new(),
        functions: user.functions,
        actions: user.actions,
    }
}

/// Roda o pipeline até depois de `tree_shake`.
fn shake(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    // Optimizer retorna MonoModule; extrair TypedModule via .inner.
    tree_shake(typed.inner)
}

/// `@test` specs devem ser removidos por tree shaking — testes não rodam
/// em binários AOT de produção.
#[test]
fn test_specs_removidos() {
    let src = r#"@test{desc: "soma basica", args: (3, 4)}
action soma (a::Int, b::Int) => Int
    + a b
soma!(1, 2)"#;
    let shaken = shake(src);
    let soma = shaken
        .actions
        .iter()
        .find(|a| a.name == "soma")
        .expect("action soma deve ser mantida");
    assert!(soma.tests.is_empty(), "tree_shake deve remover @test specs");
}

/// Função não chamada por ninguém deve ser removida.
#[test]
fn funcao_morta_removida() {
    // `auxiliar` nunca é referenciada — deve ser removida.
    let src = r#"auxiliar :: Int => Int
    lambda n: + n 1
principal :: Int => Int
    lambda n: + n 2
principal 10"#;
    let shaken = shake(src);
    assert!(
        shaken.functions.iter().any(|f| f.name == "principal"),
        "principal deve ser mantida (entry point)"
    );
    assert!(
        !shaken.functions.iter().any(|f| f.name == "auxiliar"),
        "auxiliar deve ser removida (função morta)"
    );
}

/// Função alcançada transitivamente (A chama B, B chama C, entry chama A)
/// deve ser mantida. Tree shaking deve seguir a cadeia de chamadas.
#[test]
fn funcao_transitiva_mantida() {
    let src = r#"folha :: Int => Int
    lambda n: + n 1
meio :: Int => Int
    lambda n: folha n
topo :: Int => Int
    lambda n: meio n
topo 10"#;
    let shaken = shake(src);
    assert!(
        shaken.functions.iter().any(|f| f.name == "topo"),
        "topo deve ser mantida (entry point)"
    );
    assert!(
        shaken.functions.iter().any(|f| f.name == "meio"),
        "meio deve ser mantida (alcance transitivo)"
    );
    assert!(
        shaken.functions.iter().any(|f| f.name == "folha"),
        "folha deve ser mantida (alcance transitivo)"
    );
}
