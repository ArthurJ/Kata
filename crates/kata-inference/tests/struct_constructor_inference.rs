//! Testes de inference de smart constructor de struct.

use kata_core::InterfaceRegistry;
use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

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
    }
}

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).unwrap()
}

#[test]
fn struct_constructor_sintetizado() {
    let src = "data Pessoa (nome::Text idade::Int)\nlet p := Pessoa \"João\" 30\np";
    let typed = infer_src(src);

    // Pessoa está no dispatch_table
    assert!(
        typed.dispatch_table.has_function("Pessoa"),
        "Pessoa deve estar no dispatch_table"
    );

    // Existe TypedFunction com nome Pessoa
    let constructor = typed.functions.iter().find(|f| f.name == "Pessoa");
    assert!(
        constructor.is_some(),
        "Pessoa deve estar em typed.functions"
    );
    let c = constructor.unwrap();
    assert_eq!(c.param_types, vec![Ty::text(), Ty::int()]);
    assert_eq!(c.ret_ty, Ty::Struct("Pessoa".into()));

    // Body é StructConstruct
    let body = &c.clauses[0].body.node;
    assert!(
        matches!(
            &body.kind,
            TypedExprKind::StructConstruct { struct_name, .. } if struct_name == "Pessoa"
        ),
        "body deve ser StructConstruct, encontrado {:?}",
        body.kind
    );
}

#[test]
fn struct_sem_campos_nao_tem_constructor() {
    // data Vazio () — tipo opaco, não ganha construtor
    let src = "data Vazio ()\nVazio";
    let typed = infer_src(src);
    assert!(
        !typed.dispatch_table.has_function("Vazio"),
        "Vazio (sem campos) não deve ter smart constructor"
    );
    assert!(
        !typed.functions.iter().any(|f| f.name == "Vazio"),
        "Vazio não deve estar em typed.functions"
    );
}

#[test]
fn struct_aninhada_tem_constructor() {
    let src = "data Endereco (rua::Text)\ndata Pessoa (nome::Text end::Endereco)\nlet p := Pessoa \"João\" (Endereco \"Rua A\")\np";
    let typed = infer_src(src);

    // Ambos construtores estão no dispatch_table
    assert!(typed.dispatch_table.has_function("Endereco"));
    assert!(typed.dispatch_table.has_function("Pessoa"));

    // Pessoa tem 2 campos: Text, Struct("Endereco")
    let c = typed.functions.iter().find(|f| f.name == "Pessoa").unwrap();
    assert_eq!(
        c.param_types,
        vec![Ty::text(), Ty::Struct("Endereco".into())]
    );
    assert_eq!(c.ret_ty, Ty::Struct("Pessoa".into()));
}
