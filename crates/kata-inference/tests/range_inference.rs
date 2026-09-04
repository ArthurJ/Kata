//! Testes de inferência de ranges (DoDs 24-26, step defaults, step neutro).

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

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
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
        refines_registry,
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

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn entry(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── DoD 24: `[0..1..10]` infere `Range(Int)` ──────────────────────────

#[test]
fn dod24_range_lit_infere_range_int() {
    let typed = infer_src("[0..1..10]");
    let e = entry(&typed);
    assert!(
        matches!(
            &e.kind,
            TypedExprKind::RangeLit {
                inclusive: false,
                elem_ty,
                ..
            } if *elem_ty == Ty::int()
        ),
        "entry deve ser RangeLit(Int, inclusive=false), encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::Range(Box::new(Ty::int())));
}

// ── DoD 25: `[0..1..=10]` infere `Range(Int)` com `inclusive=true` ────

#[test]
fn dod25_range_lit_inclusive() {
    let typed = infer_src("[0..1..=10]");
    let e = entry(&typed);
    assert!(
        matches!(
            &e.kind,
            TypedExprKind::RangeLit {
                inclusive: true,
                elem_ty,
                ..
            } if *elem_ty == Ty::int()
        ),
        "entry deve ser RangeLit(Int, inclusive=true), encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::Range(Box::new(Ty::int())));
}

// ── DoD 26: `[0.0..0.1..1.0]` infere `Range(Float)` ───────────────────

#[test]
fn dod26_range_lit_float() {
    let typed = infer_src("[0.0..0.1..1.0]");
    let e = entry(&typed);
    assert!(
        matches!(
            &e.kind,
            TypedExprKind::RangeLit {
                inclusive: false,
                elem_ty,
                ..
            } if *elem_ty == Ty::float()
        ),
        "entry deve ser RangeLit(Float, inclusive=false), encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::Range(Box::new(Ty::float())));
}

// ── Step default: `[0..10]` infere Range(Int) com step=IntLit(1) ──

#[test]
fn range_step_default_int_infers_range_int() {
    let typed = infer_src("[0..10]");
    let e = entry(&typed);
    assert!(
        matches!(
            &e.kind,
            TypedExprKind::RangeLit {
                inclusive: false,
                elem_ty,
                ..
            } if *elem_ty == Ty::int()
        ),
        "entry deve ser RangeLit(Int, inclusive=false), encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::Range(Box::new(Ty::int())));
}

#[test]
fn range_step_default_int_step_is_literal_one() {
    // [0..10] deve ter step = IntLit { text: "1" } no TAST
    let typed = infer_src("[0..10]");
    let e = entry(&typed);
    if let TypedExprKind::RangeLit { step, .. } = &e.kind {
        assert!(
            matches!(&step.node.kind, TypedExprKind::IntLit { text } if text == "1"),
            "step deve ser IntLit(\"1\"), encontrado {:?}",
            step.node.kind
        );
    } else {
        panic!("expected RangeLit, got {:?}", e.kind);
    }
}

#[test]
fn range_step_default_inclusive_int() {
    let typed = infer_src("[0..=10]");
    let e = entry(&typed);
    assert!(
        matches!(
            &e.kind,
            TypedExprKind::RangeLit {
                inclusive: true,
                elem_ty,
                ..
            } if *elem_ty == Ty::int()
        ),
        "entry deve ser RangeLit(Int, inclusive=true), encontrado {:?}",
        e.kind
    );
}

#[test]
fn range_step_default_float_infers_range_float() {
    let typed = infer_src("[0.0..10.0]");
    let e = entry(&typed);
    assert!(
        matches!(
            &e.kind,
            TypedExprKind::RangeLit {
                inclusive: false,
                elem_ty,
                ..
            } if *elem_ty == Ty::float()
        ),
        "entry deve ser RangeLit(Float, inclusive=false), encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::Range(Box::new(Ty::float())));
}

#[test]
fn range_step_default_float_step_is_literal_one() {
    // [0.0..10.0] deve ter step = FloatLit { text: "1.0" } no TAST
    let typed = infer_src("[0.0..10.0]");
    let e = entry(&typed);
    if let TypedExprKind::RangeLit { step, .. } = &e.kind {
        assert!(
            matches!(&step.node.kind, TypedExprKind::FloatLit { text } if text == "1.0"),
            "step deve ser FloatLit(\"1.0\"), encontrado {:?}",
            step.node.kind
        );
    } else {
        panic!("expected RangeLit, got {:?}", e.kind);
    }
}

// ── Step neutro é erro de compile-time ─────────────────────────────────

#[test]
fn range_step_neutral_int_is_error() {
    let err = infer_src_err("[0..0..10]");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("TypeMismatch"),
        "step neutro Int deve falhar com TypeMismatch, encontrado {msg}"
    );
    assert!(
        msg.contains("degenerado"),
        "mensagem deve mencionar 'degenerado', encontrado {msg}"
    );
}

#[test]
fn range_step_neutral_float_is_error() {
    let err = infer_src_err("[0.0..0.0..10.0]");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("TypeMismatch"),
        "step neutro Float deve falhar com TypeMismatch, encontrado {msg}"
    );
    assert!(
        msg.contains("degenerado"),
        "mensagem deve mencionar 'degenerado', encontrado {msg}"
    );
}

#[test]
fn range_step_default_int_is_not_neutral() {
    // [0..10] — step default = 1, não neutro. Deve passar.
    let typed = infer_src("[0..10]");
    let e = entry(&typed);
    assert_eq!(e.ty, Ty::Range(Box::new(Ty::int())));
}

#[test]
fn range_step_explicit_nonzero_int_is_not_neutral() {
    // [0..2..10] — step explícito = 2, não neutro. Deve passar.
    let typed = infer_src("[0..2..10]");
    let e = entry(&typed);
    assert_eq!(e.ty, Ty::Range(Box::new(Ty::int())));
}

#[test]
fn range_step_explicit_nonzero_float_is_not_neutral() {
    // [0.0..0.5..10.0] — step explícito = 0.5, não neutro. Deve passar.
    let typed = infer_src("[0.0..0.5..10.0]");
    let e = entry(&typed);
    assert_eq!(e.ty, Ty::Range(Box::new(Ty::float())));
}
