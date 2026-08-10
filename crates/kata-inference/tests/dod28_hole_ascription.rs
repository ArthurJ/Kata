//! DoD 28 — Holes com ascription de tipo.
//!
//! `_::Int` em posição de argumento fornece o tipo do hole diretamente,
//! sem precisar de partial dispatch. O desugar preserva a ascription:
//! `+ 10 _::Int` → `lambda __hole_0: + 10 (__hole_0::Int)`.
//! O typeck usa o tipo anotado diretamente.

use kata_core::ty::{PrimTy, Ty};
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

// ── Casos do DoD 28 ────────────────────────────────────────────────

/// `+ 10 _::Int` — hole com ascription Int.
/// O lambda deve ter tipo `Function([Int], Int)` via ascription direta,
/// sem precisar de partial dispatch (embora partial dispatch também funcionaria aqui).
#[test]
fn hole_ascription_int_direct() {
    let tmod = infer_src("+ 10 _::Int");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "+ 10 _::Int deve ter tipo Int -> Int"
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

/// `+ _::Float 10.0` — hole com ascription Float no primeiro arg.
/// O lambda deve ter tipo `Function([Float], Float)`.
#[test]
fn hole_ascription_float_first_arg() {
    let tmod = infer_src("+ _::Float 10.0");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::float()], Box::new(Ty::float())),
        "+ _::Float 10.0 deve ter tipo Float -> Float"
    );
}

/// `+ _::Int _::Float` — cross-type overload Int Float => Float agora existe.
/// O primeiro arg é Int (via ascription), o segundo é Float.
/// Deve despachar para o overload [Int, Float] → Float.
#[test]
fn hole_ascription_mixed_types_no_overload() {
    let tmod = infer_src("+ _::Int _::Float");
    let entry = entry_typed(&tmod);
    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int(), Ty::float()], Box::new(Ty::float())),
        "+ _::Int _::Float deve ter tipo (Int, Float) -> Float via cross-type overload"
    );
}

/// `+ _::Rational _::Rational` — ambos holes com ascription Rational.
/// Deve despachar para o overload Rational. Lambda tem 2 parâmetros.
#[test]
fn hole_ascription_rational_both() {
    let tmod = infer_src("+ _::Rational _::Rational");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(
            vec![Ty::Prim(PrimTy::Rational), Ty::Prim(PrimTy::Rational)],
            Box::new(Ty::Prim(PrimTy::Rational))
        ),
        "+ _::Rational _::Rational deve ter tipo (Rational, Rational) -> Rational"
    );
}

/// `* _::Int 5` — multiplicação com hole ascription Int.
#[test]
fn hole_ascription_times_int() {
    let tmod = infer_src("* _::Int 5");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "* _::Int 5 deve ter tipo Int -> Int"
    );
}

/// `let f := + 10 _::Int; f 5` — aplicação do lambda tipado via ascription.
#[test]
fn hole_ascription_then_apply() {
    let tmod = infer_src("let f := + 10 _::Int\nf 5");
    let entry = entry_typed(&tmod);

    assert_eq!(entry.ty, Ty::int(), "f 5 deve retornar Int");
}

/// `_::Int` sem contexto de dispatch — hole sozinho com ascription.
/// `let f := _::Int; f 42` — hole como valor sem Apply.
/// O desugar não gera lambda para hole fora de Apply.
/// Esperado: o hole `_::Int` como entry expression deve falhar
/// (Hole deve ter sido desugared em Apply, mas sozinho não há Apply).
#[test]
fn hole_ascription_standalone_errors() {
    let err = infer_src_err("_::Int");
    eprintln!("Got expected error: {err:?}");
}
