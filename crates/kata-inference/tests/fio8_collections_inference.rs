//! Fio 8 Fase 5 — Testes de inferência de coleções (DoDs 22-32).
//!
//! Verifica:
//! 22. `[1 2 3]` infere `List(Int)`
//! 23. `{1 2 3}` infere `Array(Int)`
//! 24. `[0..1..10]` infere `Range(Int)`
//! 25. `[0..1..=10]` infere `Range(Int)` com `inclusive=true`
//! 26. `[0.0..0.1..1.0]` infere `Range(Float)`
//! 27. `[]` infere `List(InferVar)`
//! 28. `for x in [1 2 3]` define `x: Int` no escopo do body
//! 29. `[h : t]` pattern match em List: `h: Int, t: List(Int)`
//! 30. `arr.0` em Array desugara para `at arr 0` → `Result::(Int, Err)`
//! 31. `len (10, 20)` → `2` (síntese compile-time)
//! 32. `3 in {1 2 3}` infere `Boolean`

use kata_core::InterfaceRegistry;
use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, TypedPattern, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

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
    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
        functions: user.functions,
        actions: user.actions,
    }
}

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn entry(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── DoD 22: `[1 2 3]` infere `List(Int)` ──────────────────────────────

#[test]
fn dod22_list_lit_infere_list_int() {
    let typed = infer_src("[1 2 3]");
    let e = entry(&typed);
    assert!(
        matches!(&e.kind, TypedExprKind::ListLit { elements } if elements.len() == 3),
        "entry deve ser ListLit com 3 elementos, encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::List(Box::new(Ty::int())));
}

// ── DoD 23: `{1 2 3}` infere `Array(Int)` ─────────────────────────────

#[test]
fn dod23_array_lit_infere_array_int() {
    let typed = infer_src("{1 2 3}");
    let e = entry(&typed);
    assert!(
        matches!(&e.kind, TypedExprKind::ArrayLit { elements } if elements.len() == 3),
        "entry deve ser ArrayLit com 3 elementos, encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::Array(Box::new(Ty::int())));
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

// ── DoD 27: `[]` infere `List(InferVar)` ──────────────────────────────

#[test]
fn dod27_empty_list_infere_list_infer_var() {
    let typed = infer_src("[]");
    let e = entry(&typed);
    assert!(
        matches!(&e.kind, TypedExprKind::ListLit { elements } if elements.is_empty()),
        "entry deve ser ListLit vazia, encontrado {:?}",
        e.kind
    );
    // O tipo deve ser List(InferVar(N)) — verificamos que é List de algo.
    assert!(
        matches!(&e.ty, Ty::List(_)),
        "tipo deve ser List(_), encontrado {:?}",
        e.ty
    );
}

// ── DoD 28: `for x in [1 2 3]` define `x: Int` no escopo do body ──────
//
// `for` só existe em Action body. Criamos uma action com `for x in [1 2 3]`
// e verificamos que var_ty = Int no ForIn.

#[test]
fn dod28_for_in_defines_x_int() {
    let src = "action iterar -> Int\n    var total := 0\n    for x in [1 2 3]\n        total := x\n    return total\n0";
    let typed = infer_src(src);
    // O for deve estar no body da action.
    let action = &typed.actions[0];
    // Procura o ForIn no body da action.
    let for_in = action
        .body
        .iter()
        .find(|s| matches!(&s.node.kind, TypedExprKind::ForIn { .. }))
        .expect("action body deve conter ForIn");
    match &for_in.node.kind {
        TypedExprKind::ForIn {
            var_name, var_ty, ..
        } => {
            assert_eq!(var_name, "x", "var_name deve ser 'x'");
            assert_eq!(*var_ty, Ty::int(), "var_ty deve ser Int");
        }
        other => panic!("esperado ForIn, encontrado {other:?}"),
    }
}

// ── DoD 29: `[h : t]` pattern match em List: `h: Int, t: List(Int)` ──

#[test]
fn dod29_pattern_cons_match() {
    let src = "let lst := [1 2 3]\nmatch lst\n    [h : t]: h\n    otherwise: 0";
    let typed = infer_src(src);
    let e = entry(&typed);
    // O entry deve ser um Match com um arm Cons.
    match &e.kind {
        TypedExprKind::Match { arms, .. } => {
            assert!(!arms.is_empty(), "deve ter ao menos 1 arm");
            // O primeiro arm deve ter pattern Cons com head: Int, tail: List(Int).
            let arm = &arms[0];
            let pattern = arm
                .pattern
                .as_ref()
                .expect("arm deve ter pattern (não otherwise)");
            match &pattern.node {
                TypedPattern::Cons { head, tail } => {
                    match &head.node {
                        TypedPattern::Ident { name, ty } => {
                            assert_eq!(name, "h");
                            assert_eq!(*ty, Ty::int(), "head deve ser Int");
                        }
                        other => panic!("head do Cons deve ser Ident, encontrado {other:?}"),
                    }
                    match &tail.node {
                        TypedPattern::Ident { name, ty } => {
                            assert_eq!(name, "t");
                            assert_eq!(
                                *ty,
                                Ty::List(Box::new(Ty::int())),
                                "tail deve ser List(Int)"
                            );
                        }
                        other => panic!("tail do Cons deve ser Ident, encontrado {other:?}"),
                    }
                }
                other => panic!("pattern deve ser Cons, encontrado {other:?}"),
            }
        }
        other => panic!("entry deve ser Match, encontrado {other:?}"),
    }
}

// ── DoD 30: `arr.0` em Array desugara para `at arr 0` → `Result::(Int, Err)`

#[test]
fn dod30_dot_n_array_desugars_to_at() {
    let src = "let arr := {1 2 3}\narr.0";
    let typed = infer_src(src);
    let e = entry(&typed);
    // O entry deve ser um Closure (call de `at`) com ffi_symbol.
    match &e.kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            // callee deve ser Ident("at")
            match &callee.node.kind {
                TypedExprKind::Ident { name } => {
                    assert_eq!(name, "at", "callee deve ser 'at'");
                }
                other => panic!("callee deve ser Ident(\"at\"), encontrado {other:?}"),
            }
            // 2 args: receptor + índice
            assert_eq!(args.len(), 2, "deve ter 2 args (receptor + índice)");
            // O índice deve ser IntLit(0)
            match &args[1].node.kind {
                TypedExprKind::IntLit { text } => {
                    assert_eq!(text, "0", "índice deve ser 0");
                }
                other => panic!("segundo arg deve ser IntLit(0), encontrado {other:?}"),
            }
            // ffi_symbol deve ser Some (kata_rt_array_get_checked)
            assert!(
                ffi_symbol.is_some(),
                "ffi_symbol deve ser Some (dispatch INDEXABLE)"
            );
        }
        other => panic!("entry deve ser Closure (dispatch `at`), encontrado {other:?}"),
    }
    // O tipo deve ser Result::(Int, Err)
    let expected = Ty::Generic("Result".into(), vec![Ty::int(), Ty::Struct("Err".into())]);
    assert_eq!(
        e.ty, expected,
        "tipo deve ser Result::(Int, Err), encontrado {:?}",
        e.ty
    );
}

// ── DoD 31: `len (10, 20)` → `2` (síntese compile-time) ───────────────

#[test]
fn dod31_len_tuple_sintese_compile_time() {
    let typed = infer_src("len (10, 20)");
    let e = entry(&typed);
    match &e.kind {
        TypedExprKind::IntLit { text } => {
            assert_eq!(text, "2", "len (10, 20) deve sintetizar IntLit(2)");
        }
        other => panic!("entry deve ser IntLit(2), encontrado {other:?}"),
    }
    assert_eq!(e.ty, Ty::int(), "tipo deve ser Int");
}

// ── DoD 32: `3 in {1 2 3}` infere `Boolean` ───────────────────────────

#[test]
fn dod32_in_operator_infere_boolean() {
    let typed = infer_src("3 in {1 2 3}");
    let e = entry(&typed);
    assert!(
        matches!(&e.kind, TypedExprKind::In { .. }),
        "entry deve ser In, encontrado {:?}",
        e.kind
    );
    assert_eq!(e.ty, Ty::Sum("Boolean".into()), "tipo deve ser Boolean");
}

// ── Extra: `.N` em Range é type error ─────────────────────────────────

#[test]
fn dot_n_on_range_is_type_error() {
    let err = infer_src_err("let r := [0..1..10]\nr.0");
    // Deve falhar com NotIndexable ou TypeMismatch.
    let msg = format!("{err:?}");
    assert!(
        msg.contains("NotIndexable") || msg.contains("TypeMismatch"),
        "deve falhar com NotIndexable ou TypeMismatch, encontrado {msg}"
    );
}


