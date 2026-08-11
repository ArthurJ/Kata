//! DoD 27 — Partial dispatch no DispatchTable.
//!
//! `+ 10 _` desugared vira `lambda __hole_0: + 10 __hole_0`.
//! O partial dispatch deve extrair `__hole_0: Int` do overload `[Int, Int] → Int`
//! porque `+` tem overloads Int/Float/Rational e `10` (Int) exclui Float e Rational.

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::load_prelude;

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    infer_module(&module, &prelude).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    infer_module(&module, &prelude).expect_err("inferência deve falhar")
}

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── Casos principais do DoD 27 ─────────────────────────────────────

/// `+ 10 _` → desugared para `lambda __hole_0: + 10 __hole_0`.
/// Partial dispatch com cross-type overloads: `Some(Int)` no primeiro arg
/// casa com Int Int, Int Float, Int Rational → OverloadSet (múltiplas overloads).
#[test]
fn partial_dispatch_plus_int_hole() {
    let tmod = infer_src("+ 10 _");
    let entry = entry_typed(&tmod);

    // OverloadSet: Int casa com Int Int, Int Float, Int Rational.
    // Segundo arg é None (hole), então todas essas overloads casam.
    assert_eq!(
        entry.ty,
        Ty::OverloadSet {
            name: "+".to_string(),
            overloads: vec![
                (vec![Ty::int()], Ty::int()),
                (vec![Ty::float()], Ty::float()),
                (
                    vec![Ty::Prim(kata_core::ty::PrimTy::Rational)],
                    Ty::Prim(kata_core::ty::PrimTy::Rational)
                ),
            ],
        },
        "+ 10 _ deve ter tipo OverloadSet(+, [Int], [Float], [Rational])"
    );

    match &entry.kind {
        TypedExprKind::Lambda { param_types, .. } => {
            assert_eq!(
                param_types,
                &[Ty::InferVar(0)],
                "parâmetro deve ser InferVar(0)"
            );
        }
        other => panic!("expected Lambda, got {other:?}"),
    }
}

/// `+ _ 10` — hole no primeiro arg, Int no segundo.
/// Mesmo resultado: `Function([Int], Int)`.
#[test]
fn partial_dispatch_plus_hole_int() {
    let tmod = infer_src("+ _ 10");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "+ _ 10 deve ter tipo Int -> Int"
    );
}

/// `+ 10.0 _` — Float no primeiro arg, hole no segundo.
/// Cross-type overloads: Some(Float) no primeiro arg casa com Float Float,
/// Float Rational → OverloadSet com projeções Float e Rational.
#[test]
fn partial_dispatch_plus_float_hole() {
    let tmod = infer_src("+ 10.0 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::OverloadSet {
            name: "+".to_string(),
            overloads: vec![
                (vec![Ty::float()], Ty::float()),
                (vec![Ty::Prim(kata_core::ty::PrimTy::Rational)], Ty::float()),
            ],
        },
        "+ 10.0 _ deve ter tipo OverloadSet(+, [Float], [Rational])"
    );
}

/// `+ _ _` — ambos args são holes. Com cross-type overloads, múltiplas
/// overloads casam (Int Int, Int Float, Float Float, etc.), então o
/// lambda recebe OverloadSet em vez de falhar.
#[test]
fn partial_dispatch_both_holes_ambiguous() {
    // `+ _ _` desugared vira `lambda __hole_0 __hole_1: + __hole_0 __hole_1`
    // Ambos params são InferVar, resolve_partial com [None, None] é ambíguo.
    // Com OverloadSet, o lambda defere e o dispatch resolve no call site.
    let tmod = infer_src("+ _ _");
    let entry = entry_typed(&tmod);
    assert!(
        matches!(&entry.ty, Ty::OverloadSet { name, .. } if name == "+"),
        "+ _ _ deve produzir OverloadSet(+), got {:?}",
        entry.ty
    );
}

/// `- 10 _` — subtração com hole. Cross-type overloads fazem
/// `Some(Int)` casar com Int Int, Int Float, Int Rational → OverloadSet.
#[test]
fn partial_dispatch_minus_int_hole() {
    let tmod = infer_src("- 10 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::OverloadSet {
            name: "-".to_string(),
            overloads: vec![
                (vec![Ty::int()], Ty::int()),
                (vec![Ty::float()], Ty::float()),
                (
                    vec![Ty::Prim(kata_core::ty::PrimTy::Rational)],
                    Ty::Prim(kata_core::ty::PrimTy::Rational)
                ),
            ],
        },
        "- 10 _ deve ter tipo OverloadSet(-, [Int], [Float], [Rational])"
    );
}

/// `+ 10 _` seguido de aplicação: `let f := + 10 _; f 5`
/// O lambda deve ser `Int -> Int` e aplicação `f 5` retorna `Int`.
/// `f` é uma variável no TypeEnv (não no DispatchTable), então `ffi_symbol`
/// é `None` (call_indirect) — o codegen decide como chamar.
#[test]
fn partial_dispatch_hole_then_apply() {
    let tmod = infer_src("constant f := + 10 _\nf 5");
    let entry = entry_typed(&tmod);

    assert_eq!(entry.ty, Ty::int(), "f 5 deve retornar Int");
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            // f é call_indirect (variável no TypeEnv, não no DispatchTable).
            // ffi_symbol é None — o codegen usa call_indirect.
            assert_eq!(
                ffi_symbol, &None,
                "f é call_indirect (TypeEnv), não call direto (DispatchTable)"
            );
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}

/// `* 10 _` — multiplicação com hole. Cross-type overloads fazem
/// `Some(Int)` casar com Int Int, Int Float, Int Rational → OverloadSet.
#[test]
fn partial_dispatch_times_int_hole() {
    let tmod = infer_src("* 10 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::OverloadSet {
            name: "*".to_string(),
            overloads: vec![
                (vec![Ty::int()], Ty::int()),
                (vec![Ty::float()], Ty::float()),
                (
                    vec![Ty::Prim(kata_core::ty::PrimTy::Rational)],
                    Ty::Prim(kata_core::ty::PrimTy::Rational)
                ),
            ],
        },
        "* 10 _ deve ter tipo OverloadSet(*, [Int], [Float], [Rational])"
    );
}

/// `+ 10::Rational _` com Rational no primeiro arg.
/// Cross-type overloads: Some(Rational) casa com Rational Rational e
/// Rational Float → OverloadSet com projeções Rational e Rational (de Float).
#[test]
fn partial_dispatch_plus_rational_hole() {
    let tmod = infer_src("+ 10::Rational _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::OverloadSet {
            name: "+".to_string(),
            overloads: vec![
                (
                    vec![Ty::Prim(kata_core::ty::PrimTy::Rational)],
                    Ty::Prim(kata_core::ty::PrimTy::Rational)
                ),
                (vec![Ty::float()], Ty::Prim(kata_core::ty::PrimTy::Rational)),
            ],
        },
        "+ 10::Rational _ deve ter tipo OverloadSet(+, [Rational], [Float])"
    );
}
