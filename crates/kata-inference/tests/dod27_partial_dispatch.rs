//! DoD 27 — Partial dispatch no DispatchTable.
//!
//! `+ 10 _` desugared vira `lambda __hole_0: + 10 __hole_0`.
//! O partial dispatch deve extrair `__hole_0: Int` do overload `[Int, Int] → Int`
//! porque `+` tem overloads Int/Float/Rational e `10` (Int) exclui Float e Rational.

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{load_stdlib_for_tests, merge_two, resolve};

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_two(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

#[allow(dead_code)]
fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    infer_module(&module, &prelude).expect_err("inferência deve falhar")
}

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── Casos principais do DoD 27 ─────────────────────────────────────

/// `+ 10 _` → desugared para `lambda __hole_0: + 10 __hole_0`.
/// Some(Int) no primeiro arg: casa com Int Int (same-type). As overloads
/// cross-type (Float Int, Rational Int) têm Float/Rational no primeiro param,
/// não casam com Int no primeiro arg sem swap. Resultado: Function([Int], Int).
#[test]
fn partial_dispatch_plus_int_hole() {
    let tmod = infer_src("+ 10 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "+ 10 _ deve ter tipo Int -> Int (cross-type só casa via swap, partial dispatch não faz swap)"
    );

    match &entry.kind {
        TypedExprKind::Lambda { param_types, .. } => {
            assert_eq!(
                param_types,
                &[Ty::int()],
                "parâmetro deve ser Int (resolvido por partial dispatch)"
            );
        }
        other => panic!("expected Lambda, got {other:?}"),
    }
}

/// `+ _ 10` — hole no primeiro arg, Int no segundo.
/// Cross-type overloads via @commutative swap: Int no segundo arg casa com
/// Int Int, Float Int (swap→Int Float), Rational Int (swap→Int Rational).
/// Resultado: OverloadSet com Int, Float, Rational.
#[test]
fn partial_dispatch_plus_hole_int() {
    let tmod = infer_src("+ _ 10");
    let entry = entry_typed(&tmod);

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
        "+ _ 10 deve ter tipo OverloadSet(+, [Int], [Float], [Rational])"
    );
}

/// `+ 10.0 _` — Float no primeiro arg, hole no segundo.
/// Cross-type overloads: Some(Float) no primeiro arg casa com Float Float,
/// Float Int, Float Rational → OverloadSet com projeções Float, Int e Rational.
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
                (vec![Ty::int()], Ty::float()),
                (vec![Ty::Prim(kata_core::ty::PrimTy::Rational)], Ty::float()),
            ],
        },
        "+ 10.0 _ deve ter tipo OverloadSet(+, [Float], [Int], [Rational]) — todas retornam Float"
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

/// `- 10 _` — subtração com hole. Some(Int) no primeiro arg casa com
/// Int Int (same-type) apenas. Cross-type (Float Int, Rational Int) não
/// casam no primeiro arg sem swap. Resultado: Function([Int], Int).
#[test]
fn partial_dispatch_minus_int_hole() {
    let tmod = infer_src("- 10 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "- 10 _ deve ter tipo Int -> Int (cross-type só casa via swap)"
    );
}

/// `+ 10 _` seguido de aplicação: `let f := + 10 _; f 5`
/// O lambda deve ser `Int -> Int` e aplicação `f 5` retorna `Int`.
/// `f` é uma variável no TypeEnv (não no DispatchTable), então `ffi_symbol`
/// é `None` (call_indirect) — o codegen decide como chamar.
#[test]
fn partial_dispatch_hole_then_apply() {
    // Migrado de `constant f := + 10 _` — sections produzem lambdas,
    // que não são permitidas em `constant`. Usa sintaxe de função nomeada.
    let tmod = infer_src("f :: Int => Int\nlambda x: + 10 x\nf 5");
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

/// `* 10 _` — multiplicação com hole. Some(Int) no primeiro arg casa com
/// Int Int (same-type) apenas. Cross-type (Float Int, Rational Int) não
/// casam no primeiro arg sem swap. Resultado: Function([Int], Int).
#[test]
fn partial_dispatch_times_int_hole() {
    let tmod = infer_src("* 10 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "* 10 _ deve ter tipo Int -> Int (cross-type só casa via swap)"
    );
}

/// `+ 10::Rational _` com Rational no primeiro arg.
/// Cross-type overloads: Some(Rational) no primeiro arg casa com Rational
/// Rational, Rational Int, Rational Float → OverloadSet com projeções
/// Rational (same-type), Rational (de Int), Rational (de Float).
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
                (vec![Ty::int()], Ty::Prim(kata_core::ty::PrimTy::Rational)),
                (vec![Ty::float()], Ty::Prim(kata_core::ty::PrimTy::Rational)),
            ],
        },
        "+ 10::Rational _ deve ter tipo OverloadSet(+, [Rational], [Int], [Float]) — todas retornam Rational"
    );
}
