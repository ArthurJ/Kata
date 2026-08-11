//! Testes E2E da inferência bidirecional — DoDs 27-31.
//!
//! Valida os 5 mecanismos de inferência de lambda:
//! - DoD 27: partial dispatch (`+ 10 _`)
//! - DoD 28: hole com ascription (`_::Int`)
//! - DoD 29: hint top-down (`(lambda ...)::(Int -> Int)`)
//! - DoD 30: LambdaInferenceFail (erro distinto)
//! - DoD 31: apply de lambda inline (`(lambda x: + x 1) 42`)

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{load_prelude, merge_two, resolve};

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_two(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    infer_module(&module, &prelude).expect_err("deve falhar")
}

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── DoD 27: Partial dispatch ─────────────────────────────────────

/// `+ 10 _` desugared vira `lambda __hole0: + 10 __hole0`.
/// Com cross-type overloads, `Some(Int)` no primeiro arg casa com
/// Int Int, Int Float, Int Rational → OverloadSet (múltiplas overloads).
#[test]
fn dod27_partial_dispatch_resolves_int() {
    let tmod = infer_src("+ 10 _");
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
        "+ 10 _ deve ser OverloadSet(+, [Int], [Float], [Rational])"
    );
}

// ── DoD 28: Hole com ascription ────────────────────────────────────

/// `+ 10 _::Int` — hole com ascription fornece tipo diretamente.
/// Também desugared para lambda Int -> Int.
#[test]
fn dod28_hole_ascription_provides_type() {
    let tmod = infer_src("+ 10 _::Int");
    let entry = entry_typed(&tmod);
    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "+ 10 _::Int deve ser lambda Int -> Int"
    );
}

// ── DoD 29: Hint top-down ─────────────────────────────────────────

/// `(lambda x: + x 1)::(Int -> Int)` — hint top-down.
#[test]
fn dod29_hint_top_down() {
    let tmod = infer_src("(lambda x: + x 1)::(Int -> Int)");
    let entry = entry_typed(&tmod);
    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "lambda deve ter tipo Int -> Int"
    );
}

// ── DoD 30: LambdaInferenceFail ───────────────────────────────────

/// `lambda x: x` sem contexto → LambdaInferenceFail.
#[test]
fn dod30_lambda_inference_fail() {
    let err = infer_src_err("lambda x: x");
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::LambdaInferenceFail { .. }
        ),
        "esperava LambdaInferenceFail, got {err:?}"
    );
}

// ── DoD 31: Apply de lambda inline ─────────────────────────────────

/// `(lambda x: + x 1) 42` — args fornecem tipos dos params.
#[test]
fn dod31_apply_lambda_inline() {
    let tmod = infer_src("(lambda x: + x 1) 42");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int(), "aplicação deve retornar Int");
}

/// `((lambda x: + x 1)::(Int -> Int)) 42` — com ascription.
#[test]
fn dod31_apply_lambda_inline_ascription() {
    let tmod = infer_src("((lambda x: + x 1)::(Int -> Int)) 42");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int(), "aplicação deve retornar Int");
}

// ── Combinações dos mecanismos ─────────────────────────────────────

/// `let f := (lambda x: + x 1)::(Int -> Int); f 5` — hint + apply via var.
#[test]
fn hint_then_apply_via_var() {
    let tmod = infer_src("f :: Int => Int\nlambda x: + x 1\nf 5");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int(), "f 5 deve retornar Int");
}

/// `let f := lambda x: + x 1; f 5` — partial dispatch resolve x: Int,
/// f tem tipo Int -> Int, f 5 despacha via TypeEnv (call_indirect).
#[test]
fn partial_dispatch_then_apply_via_var() {
    let tmod = infer_src("f :: Int => Int\nlambda x: + x 1\nf 5");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int(), "f 5 deve retornar Int");
}

/// `(lambda x: + x 1)::(Float -> Float)` com body `+ x 1.0` — hint Float.
#[test]
fn hint_float_with_float_body() {
    let tmod = infer_src("(lambda x: + x 1.0)::(Float -> Float)");
    let entry = entry_typed(&tmod);
    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::float()], Box::new(Ty::float())),
        "lambda deve ter tipo Float -> Float"
    );
}
