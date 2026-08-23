//! Testes E2E de tree shaking — removem código morto do TypedModule.
//!
//! Pipeline completo até tree_shake: lex → parse → resolve → infer →
//! monomorphize → optimize → tree_shake. Verifica que funções/actions
//! não alcançadas e `@test` specs são removidos, e que funções alcançadas
//! transitivamente são mantidas.

use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
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

/// Roda o pipeline até depois de `tree_shake`.
fn shake(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
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

/// Action referenciada como first-class value (Ident com ty: Ty::Action)
/// deve ser preservada pelo tree shaking — pode ser invocada indiretamente.
#[test]
fn action_referenciada_como_valor_mantida() {
    // `worker` é referenciada via `let f := worker` (Ident com ty: Ty::Action),
    // mas nunca chamada diretamente com `worker!()`. Tree shaking deve
    // preservar `worker` porque `f!(42)` pode invocá-la indiretamente.
    let src = r#"action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    let f := worker
    f!(42)

main!()"#;
    let shaken = shake(src);
    assert!(
        shaken.actions.iter().any(|a| a.name == "main"),
        "main deve ser mantida (entry point)"
    );
    assert!(
        shaken.actions.iter().any(|a| a.name == "worker"),
        "worker deve ser mantida — referenciada como first-class Action (Ident com Ty::Action)"
    );
}

/// Action NÃO referenciada (nem como first-class, nem invocada) deve ser
/// removida pelo tree shaking — DoD 15 do PRD first-class actions.
#[test]
fn action_nao_referenciada_removida() {
    // `unused_worker` nunca é referenciada — nem com `!()`, nem como Ident.
    // `used_worker` é referenciada via `let f := used_worker`.
    let src = r#"action used_worker (n :: Int) => Unit
    echo!(n)

action unused_worker (n :: Int) => Unit
    echo!(+ n 100)

action main => Unit
    let f := used_worker
    f!(42)

main!()"#;
    let shaken = shake(src);
    assert!(
        shaken.actions.iter().any(|a| a.name == "main"),
        "main deve ser mantida (entry point)"
    );
    assert!(
        shaken.actions.iter().any(|a| a.name == "used_worker"),
        "used_worker deve ser mantida — referenciada como first-class Action"
    );
    assert!(
        !shaken.actions.iter().any(|a| a.name == "unused_worker"),
        "unused_worker deve ser removida — nunca referenciada (DoD 15)"
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
