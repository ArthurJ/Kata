//! Testes de inference de DotAccess (field access + index access).

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
        interface_registry: { let mut ir = prelude.interface_registry.clone(); ir.merge(user.interface_registry.clone()); ir },
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

/// `pessoa.nome` → FieldAccess com field_index = 0, ty = Text.
#[test]
fn field_access_em_struct() {
    let src = "data Pessoa (nome::Text idade::Int)\nlet p := Pessoa \"João\" 30\np.nome";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::FieldAccess {
                struct_name,
                field_name,
                field_index: 0,
                ..
            } if struct_name == "Pessoa" && field_name == "nome"
        ),
        "entry deve ser FieldAccess(Pessoa.nome, idx=0), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::text());
}

/// `pessoa.idade` → FieldAccess com field_index = 1, ty = Int.
#[test]
fn field_access_segundo_campo() {
    let src = "data Pessoa (nome::Text idade::Int)\nlet p := Pessoa \"João\" 30\np.idade";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::FieldAccess {
                field_name,
                field_index: 1,
                ..
            } if field_name == "idade"
        ),
        "entry deve ser FieldAccess(Pessoa.idade, idx=1), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::int());
}

/// `t.0` em tupla de 3 elementos → IndexAccess, element_index = 0, ty = Int.
#[test]
fn index_access_primeiro_elemento() {
    let src = "(10, 20, 30).0";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::IndexAccess {
                element_index: 0,
                index: 0,
                ..
            }
        ),
        "entry deve ser IndexAccess(idx=0), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::int());
}

/// `t.(-1)` → IndexAccess com index = -1, element_index = 2 (len-1).
#[test]
fn index_access_negativo_ultimo() {
    let src = "(10, 20, 30).(-1)";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::IndexAccess {
                index: -1,
                element_index: 2,
                ..
            }
        ),
        "entry deve ser IndexAccess(idx=-1, resolved=2), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::int());
}

/// `t.(-2)` → IndexAccess com index = -2, element_index = 1.
#[test]
fn index_access_negativo_penultimo() {
    let src = "(10, 20, 30).(-2)";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::IndexAccess {
                index: -2,
                element_index: 1,
                ..
            }
        ),
        "entry deve ser IndexAccess(idx=-2, resolved=1), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::int());
}

/// `t.5` em tupla de 3 elementos → erro IndexOutOfBounds.
#[test]
fn index_access_out_of_bounds() {
    let src = "(10, 20, 30).5";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(
        result.is_err(),
        "t.5 em tupla de 3 elementos deve falhar com IndexOutOfBounds"
    );
}

/// `t.nome` em tupla → erro FieldAccessOnTuple.
#[test]
fn field_access_em_tupla_da_erro() {
    let src = "(10, 20, 30).nome";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(
        result.is_err(),
        "t.nome em tupla deve falhar com FieldAccessOnTuple"
    );
}

/// `p.0` em struct → erro IndexAccessOnStruct.
#[test]
fn index_access_em_struct_da_erro() {
    let src = "data Pessoa (nome::Text)\nlet p := Pessoa \"João\"\np.0";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(
        result.is_err(),
        "p.0 em struct deve falhar com IndexAccessOnStruct"
    );
}

/// `42.nome` → erro NotIndexable.
#[test]
fn dot_access_em_literal_da_erro() {
    let src = "42.nome";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(result.is_err(), "42.nome deve falhar com NotIndexable");
}

/// Struct aninhada: `pessoa.endereco.rua` — field access encadeado.
#[test]
fn field_access_encadeado() {
    let src = "data Endereco (rua::Text cidade::Text)\ndata Pessoa (nome::Text end::Endereco)\nlet p := Pessoa \"João\" (Endereco \"Rua A\" \"Cidade B\")\np.end.rua";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    // outer: FieldAccess(Endereco.rua, idx=0) → ty = Text
    // inner: FieldAccess(Pessoa.end, idx=1) → ty = Struct("Endereco")
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::FieldAccess {
                field_name,
                field_index: 0,
                ..
            } if field_name == "rua"
        ),
        "outer deve ser FieldAccess(Endereco.rua, idx=0), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::text());
}

/// Tupla heterogênea: `(42, "ok").1` → IndexAccess, ty = Text.
#[test]
fn index_access_tupla_heterogenea() {
    let src = "(42, \"ok\").1";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::IndexAccess {
                element_index: 1,
                ..
            }
        ),
        "entry deve ser IndexAccess(idx=1), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::text());
}

/// `t.(-3)` em tupla de 3 elementos → element_index = 0 (primeiro).
#[test]
fn index_access_negativo_primeiro() {
    let src = "(10, 20, 30).(-3)";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::IndexAccess {
                index: -3,
                element_index: 0,
                ..
            }
        ),
        "entry deve ser IndexAccess(idx=-3, resolved=0), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::int());
}

/// `t.(-4)` em tupla de 3 elementos → erro IndexOutOfBounds.
#[test]
fn index_access_negativo_out_of_bounds() {
    let src = "(10, 20, 30).(-4)";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(
        result.is_err(),
        "t.(-4) em tupla de 3 deve falhar com IndexOutOfBounds"
    );
}

/// Struct com 1 campo: field access funciona.
#[test]
fn struct_um_campo_field_access() {
    let src = "data Wrapper (valor::Int)\nlet w := Wrapper 42\nw.valor";
    let typed = infer_src(src);
    let entry = &typed.entry.node;
    assert!(
        matches!(
            &entry.kind,
            TypedExprKind::FieldAccess {
                field_name,
                field_index: 0,
                ..
            } if field_name == "valor"
        ),
        "entry deve ser FieldAccess(Wrapper.valor, idx=0), encontrado {:?}",
        entry.kind
    );
    assert_eq!(entry.ty, Ty::int());
}

/// Field access em campo inexistente → erro UnknownField.
#[test]
fn field_access_inexistente_da_erro() {
    let src = "data Pessoa (nome::Text)\nlet p := Pessoa \"João\"\np.idade";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(
        result.is_err(),
        "p.idade em struct sem campo idade deve falhar com UnknownField"
    );
}
