//! DoD 31 — Apply de lambda inline.
//!
//! `(lambda x: + x 1) 42` — args fornecem tipos dos parâmetros do lambda.
//! Síntese bottom-up: infere cada arg, usa `arg_tys` como tipos dos params.
//!
//! `((lambda x: + x 1)::(Int -> Int)) 42` — lambda com ascription aplicado.
//! O hint da ascription fornece os tipos dos params, args são verificados.

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

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

/// `(lambda x: + x 1) 42` — apply de lambda inline sem ascription.
/// Args fornecem tipos dos params: x: Int (de 42), body: + x 1 = Int.
#[test]
fn apply_lambda_inline_basic() {
    let tmod = infer_src("(lambda x: + x 1) 42");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int(), "aplicação deve retornar Int");
}

/// `(lambda x y: + x y) 10 20` — dois args.
#[test]
fn apply_lambda_inline_two_args() {
    let tmod = infer_src("(lambda x y: + x y) 10 20");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int(), "aplicação deve retornar Int");
}

/// `((lambda x: + x 1)::(Int -> Int)) 42` — apply de lambda com ascription.
#[test]
fn apply_lambda_inline_with_ascription() {
    let tmod = infer_src("((lambda x: + x 1)::(Int -> Int)) 42");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int(), "aplicação deve retornar Int");
}

/// `(lambda x: x) 42` — identidade aplicada. x: Int de 42, ret = x = Int.
#[test]
fn apply_lambda_inline_identity() {
    let tmod = infer_src("(lambda x: x) 42");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int(), "identidade deve retornar Int");
}

/// `(lambda x: + x 1) 42` — verifica que é um Closure no TAST.
#[test]
fn apply_lambda_inline_produces_closure() {
    let tmod = infer_src("(lambda x: + x 1) 42");
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
            ..
        } => {
            assert_eq!(args.len(), 1, "deve ter 1 arg");
            assert!(ffi_symbol.is_none(), "call_indirect — sem FFI symbol");
            // callee é um Lambda
            match &callee.node.kind {
                TypedExprKind::Lambda {
                    param_types,
                    ret_ty,
                    ..
                } => {
                    assert_eq!(param_types, &[Ty::int()], "x deve ser Int");
                    assert_eq!(*ret_ty, Ty::int(), "ret deve ser Int");
                }
                other => panic!("expected Lambda callee, got {other:?}"),
            }
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}

/// Aridade incorreta: `(lambda x: + x 1) 1 2` deve dar ArityMismatch.
#[test]
fn apply_lambda_inline_arity_mismatch() {
    let tokens = lex("(lambda x: + x 1) 1 2").unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let err = infer_module(&module, &prelude).expect_err("deve falhar");
    assert!(
        matches!(err, kata_diagnostics::MiddleError::ArityMismatch { .. }),
        "esperava ArityMismatch, got {err:?}"
    );
}