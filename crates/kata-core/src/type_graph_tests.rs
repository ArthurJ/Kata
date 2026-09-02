use super::*;

fn build_test_graph() -> TypeGraph {
    let mut graph = TypeGraph::new();

    // Primitivos
    graph.insert("Int", TypeKind::Primitive(PrimTy::Int), "core");
    graph.insert("Float", TypeKind::Primitive(PrimTy::Float), "core");

    // NonZero — família polimórfica
    graph.insert(
        "NonZero",
        TypeKind::Family {
            instances: vec!["Int".into(), "Float".into(), "Rational".into()],
        },
        "core",
    );
    graph.add_edge(
        "NonZero",
        TypeEdge::Instance {
            concrete: "Int".into(),
        },
        "Int",
    );
    graph.add_edge(
        "NonZero",
        TypeEdge::Instance {
            concrete: "Float".into(),
        },
        "Float",
    );

    // PositiveInt — refined concreto
    graph.insert(
        "PositiveInt",
        TypeKind::Refined {
            alias_target: "Int".into(),
            predicates: vec!["__pred_PositiveInt_0".into()],
        },
        "core",
    );
    graph.add_edge("PositiveInt", TypeEdge::Alias, "Int");

    // Altura — alias
    graph.insert(
        "Altura",
        TypeKind::Alias {
            target: "Float".into(),
        },
        "core",
    );
    graph.add_edge("Altura", TypeEdge::Alias, "Float");

    // Peso — alias de PositiveFloat (cadeia)
    graph.insert(
        "Peso",
        TypeKind::Alias {
            target: "PositiveFloat".into(),
        },
        "user",
    );
    graph.add_edge("Peso", TypeEdge::Alias, "PositiveFloat");
    graph.insert(
        "PositiveFloat",
        TypeKind::Refined {
            alias_target: "Float".into(),
            predicates: vec!["__pred_PositiveFloat_0".into()],
        },
        "core",
    );
    graph.add_edge("PositiveFloat", TypeEdge::Alias, "Float");

    // Result — enum genérico
    graph.insert(
        "Result",
        TypeKind::GenericEnum {
            type_params: vec!["T".into(), "E".into()],
        },
        "core",
    );
    graph.add_edge("Result", TypeEdge::GenericParam { name: "T".into() }, "T");
    graph.add_edge("Result", TypeEdge::GenericParam { name: "E".into() }, "E");

    // NUM — interface
    graph.insert(
        "NUM",
        TypeKind::Interface {
            supertraits: vec!["ORD".into(), "EQ".into()],
        },
        "core",
    );
    graph.add_edge("NUM", TypeEdge::Supertrait { name: "ORD".into() }, "ORD");
    graph.add_edge("NUM", TypeEdge::Supertrait { name: "EQ".into() }, "EQ");

    // Int implements NUM
    graph.add_edge(
        "Int",
        TypeEdge::Implements {
            interface: "NUM".into(),
        },
        "NUM",
    );

    // PositiveInt refines NUM
    graph.add_edge(
        "PositiveInt",
        TypeEdge::Refines {
            interface: "NUM".into(),
        },
        "NUM",
    );

    graph
}

#[test]
fn classify_family() {
    let graph = build_test_graph();
    assert!(graph.is_family("NonZero"));
    assert!(!graph.is_family("Int"));
    assert!(!graph.is_family("Result"));
}

#[test]
fn classify_generic_enum() {
    let graph = build_test_graph();
    assert!(graph.is_generic_enum("Result"));
    assert!(!graph.is_generic_enum("NonZero"));
}

#[test]
fn instances_of_family() {
    let graph = build_test_graph();
    let instances = graph.instances_of("NonZero");
    assert_eq!(instances, vec!["Int", "Float", "Rational"]);
    assert!(graph.has_instance("NonZero", "Int"));
    assert!(!graph.has_instance("NonZero", "Text"));
}

#[test]
fn alias_target_single() {
    let graph = build_test_graph();
    assert_eq!(graph.alias_target("Altura"), Some("Float"));
    assert_eq!(graph.alias_target("PositiveInt"), Some("Int"));
    assert_eq!(graph.alias_target("Int"), None);
}

#[test]
fn follow_alias_chain() {
    let graph = build_test_graph();
    // Peso → PositiveFloat → Float
    assert_eq!(graph.follow_alias("Peso"), "Float");
    // PositiveInt → Int (refined tem alias_target)
    assert_eq!(graph.follow_alias("PositiveInt"), "Int");
    // Int — não é alias, retorna ele mesmo
    assert_eq!(graph.follow_alias("Int"), "Int");
}

#[test]
fn refines_interfaces() {
    let graph = build_test_graph();
    let ifaces = graph.refines_interfaces("PositiveInt");
    assert_eq!(ifaces, vec!["NUM"]);
}

#[test]
fn implements_list() {
    let graph = build_test_graph();
    let ifaces = graph.implements("Int");
    assert_eq!(ifaces, vec!["NUM"]);
}

#[test]
fn type_implements_direct() {
    let graph = build_test_graph();
    assert!(graph.type_implements("Int", "NUM"));
    assert!(!graph.type_implements("Int", "SHOW"));
}

#[test]
fn resolve_param_app_family() {
    let graph = build_test_graph();
    let result = resolve_param_app("NonZero", &[Ty::Prim(PrimTy::Int)], &graph);
    assert_eq!(
        result,
        Some(Ty::Struct(crate::StructKey::Instance(
            "NonZero".into(),
            "Int".into()
        )))
    );
}

#[test]
fn resolve_param_app_generic_enum() {
    let graph = build_test_graph();
    let result = resolve_param_app(
        "Result",
        &[Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Text)],
        &graph,
    );
    assert_eq!(
        result,
        Some(Ty::Generic(
            "Result".into(),
            vec![Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Text)]
        ))
    );
}

#[test]
fn resolve_param_app_unknown() {
    let graph = build_test_graph();
    let result = resolve_param_app("Unknown", &[Ty::Prim(PrimTy::Int)], &graph);
    assert_eq!(result, None);
}

#[test]
fn resolve_param_app_family_wrong_concrete() {
    let graph = build_test_graph();
    // Text não é instância de NonZero
    let result = resolve_param_app("NonZero", &[Ty::Prim(PrimTy::Text)], &graph);
    assert_eq!(result, None);
}

#[test]
fn merge_graphs() {
    let mut prelude = build_test_graph();

    // User declara um novo refined
    let mut user = TypeGraph::new();
    user.insert(
        "MyRefined",
        TypeKind::Refined {
            alias_target: "Int".into(),
            predicates: vec!["__pred_MyRefined_0".into()],
        },
        "__local__",
    );

    prelude.merge(&user);
    assert!(prelude.is_refined("MyRefined"));
    assert_eq!(prelude.alias_target("MyRefined"), Some("Int"));
    // Original intacto
    assert!(prelude.is_family("NonZero"));
}
