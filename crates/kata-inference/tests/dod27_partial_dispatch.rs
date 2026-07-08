//! DoD 27 — Partial dispatch no DispatchTable.
//!
//! `+ 10 _` desugared vira `lambda __hole_0: + 10 __hole_0`.
//! O partial dispatch deve extrair `__hole_0: Int` do overload `[Int, Int] → Int`
//! porque `+` tem overloads Int/Float/Rational e `10` (Int) exclui Float e Rational.

use kata_core::ty::Ty;
use kata_inference::{infer_module, TypedExprKind};
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
/// Partial dispatch deve extrair `__hole_0: Int` e o lambda ter tipo
/// `Function([Int], Int)`.
#[test]
fn partial_dispatch_plus_int_hole() {
    let tmod = infer_src("+ 10 _");
    let entry = entry_typed(&tmod);

    // O entry deve ser um Lambda com tipo Function([Int], Int)
    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "+ 10 _ deve ter tipo Int -> Int"
    );

    match &entry.kind {
        TypedExprKind::Lambda {
            param_types,
            ret_ty,
            ..
        } => {
            assert_eq!(param_types, &[Ty::int()], "parâmetro deve ser Int");
            assert_eq!(*ret_ty, Ty::int(), "retorno deve ser Int");
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
/// Deve extrair `Float` do overload `[Float, Float] → Float`.
#[test]
fn partial_dispatch_plus_float_hole() {
    let tmod = infer_src("+ 10.0 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::float()], Box::new(Ty::float())),
        "+ 10.0 _ deve ter tipo Float -> Float"
    );
}

/// `+ _ _` — ambos args são holes. Deve ser ambíguo (3 overloads casam).
/// Como é ambíguo, o lambda recebe InferVar — não há tipo para extrair.
/// O typeck deve falhar ao tentar despachar `+ InferVar InferVar` no body.
#[test]
fn partial_dispatch_both_holes_ambiguous() {
    // `+ _ _` desugared vira `lambda __hole_0 __hole_1: + __hole_0 __hole_1`
    // Ambos params são InferVar, resolve_partial com [None, None] é ambíguo.
    // try_partial_dispatch retorna Vec::new(), params ficam InferVar.
    // O body `+ InferVar InferVar` não despacha → NoOverload ou AmbiguousDispatch.
    let err = infer_src_err("+ _ _");
    // Deve ser algum erro de dispatch — não importa qual variant específico
    eprintln!("Got expected error: {err:?}");
}

/// `- 10 _` — subtração com hole. Deve funcionar igual `+`.
#[test]
fn partial_dispatch_minus_int_hole() {
    let tmod = infer_src("- 10 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "- 10 _ deve ter tipo Int -> Int"
    );
}

/// `+ 10 _` seguido de aplicação: `let f := + 10 _; f 5`
/// O lambda deve ser `Int -> Int` e aplicação `f 5` retorna `Int`.
/// `f` é uma variável no TypeEnv (não no DispatchTable), então `ffi_symbol`
/// é `None` (call_indirect) — o codegen na Fase 9 decide como chamar.
#[test]
fn partial_dispatch_hole_then_apply() {
    let tmod = infer_src("let f := + 10 _\nf 5");
    let entry = entry_typed(&tmod);

    assert_eq!(entry.ty, Ty::int(), "f 5 deve retornar Int");
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            // f é call_indirect (variável no TypeEnv, não no DispatchTable).
            // ffi_symbol é None — o codegen usa call_indirect na Fase 9.
            assert_eq!(
                ffi_symbol, &None,
                "f é call_indirect (TypeEnv), não call direto (DispatchTable)"
            );
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}

/// `* 10 _` — multiplicação com hole. Deve extrair Int.
#[test]
fn partial_dispatch_times_int_hole() {
    let tmod = infer_src("* 10 _");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "* 10 _ deve ter tipo Int -> Int"
    );
}

/// `+ 10 _` com Rational: `+ 10::Rational _`
/// Não deve usar partial dispatch (ascription é tratada no DoD 28).
/// Aqui testamos apenas que o literal `10::Rational` resolve para Rational
/// e o partial dispatch extrai Rational do overload.
#[test]
fn partial_dispatch_plus_rational_hole() {
    let tmod = infer_src("+ 10::Rational _");
    let entry = entry_typed(&tmod);

    // 10::Rational rebaixa Int literal para Rational.
    // + tem overload [Rational, Rational] → Rational.
    // Partial dispatch deve extrair Rational.
    assert_eq!(
        entry.ty,
        Ty::Function(
            vec![Ty::Prim(kata_core::ty::PrimTy::Rational)],
            Box::new(Ty::Prim(kata_core::ty::PrimTy::Rational))
        ),
        "+ 10::Rational _ deve ter tipo Rational -> Rational"
    );
}