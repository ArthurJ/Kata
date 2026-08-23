//! DoD 30 — LambdaInferenceFail.
//!
//! Quando nenhum mecanismo (partial dispatch, ascription de hole, hint top-down)
//! fornece o tipo de um parâmetro de lambda, o typeck produz `LambdaInferenceFail`
//! em vez de criar InferVar e deixar o dispatch falhar com `NoOverload` opaco.
//!
//! Caso base: `lambda x: + x 1` sem contexto de tipo — `x` é InferVar, `+` não
//! despacha, mas o erro deve ser `LambdaInferenceFail` (não `NoOverload`).

use kata_core::ty::Ty;
use kata_diagnostics::MiddleError;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::load_stdlib_for_tests;

fn infer_src_err(src: &str) -> MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    match infer_module(&module, &prelude) {
        Ok(_) => panic!("esperava erro de inferência para: {src}"),
        Err(e) => e,
    }
}

/// `lambda x: x` — identidade sem contexto. x não participa de dispatch,
/// então partial dispatch não resolve. Deve produzir LambdaInferenceFail.
#[test]
fn identity_lambda_without_context_fails() {
    let err = infer_src_err("lambda x: x");
    assert!(
        matches!(err, MiddleError::LambdaInferenceFail { .. }),
        "esperava LambdaInferenceFail, got {err:?}"
    );
}

/// `lambda x y: + x y` — dois params sem contexto. Com cross-type overloads,
/// `+ InferVar InferVar` é ambíguo mas produz OverloadSet em vez de falhar.
/// O lambda defere e o dispatch resolve no call site.
#[test]
fn two_param_lambda_without_context_fails() {
    let tokens = lex("lambda x y: + x y").unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let tmod =
        infer_module(&module, &prelude).expect("lambda x y: + x y deve succeed com OverloadSet");
    let entry = &tmod.entry.node;
    assert!(
        matches!(&entry.ty, Ty::OverloadSet { name, .. } if name == "+"),
        "lambda x y: + x y deve produzir OverloadSet(+), got {:?}",
        entry.ty
    );
}

/// `lambda x: g x` — callee não está no DispatchTable. Partial dispatch
/// não consegue inferir. Deve produzir LambdaInferenceFail.
#[test]
fn lambda_with_unknown_callee_fails() {
    let err = infer_src_err("lambda x: g x");
    assert!(
        matches!(err, MiddleError::LambdaInferenceFail { .. }),
        "esperava LambdaInferenceFail, got {err:?}"
    );
}

/// `lambda x y: < x y` — agora succeeds com OverloadSet.
/// `<` tem múltiplas overloads (Int Int, Float Float, Rational Rational),
/// e com todos args como InferVar, múltiplas overloads casam → OverloadSet.
/// Antes do OverloadSet, isto produzia LambdaInferenceFail com detail.
#[test]
fn lambda_inference_fail_has_detail() {
    let tokens = lex("lambda x y: < x y").unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let tmod =
        infer_module(&module, &prelude).expect("lambda x y: < x y deve succeed com OverloadSet");
    let entry = &tmod.entry.node;
    assert!(
        matches!(&entry.ty, Ty::OverloadSet { name, .. } if name == "<"),
        "lambda x y: < x y deve produzir OverloadSet(<), got {:?}",
        entry.ty
    );
}

/// `(lambda x: + x 1)::(Int -> Int)` — COM hint top-down, deve succeed.
/// Este teste confirma que o hint previne o LambdaInferenceFail.
#[test]
fn lambda_with_hint_does_not_fail() {
    let tokens = lex("(lambda x: + x 1)::(Int -> Int)").unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let result = infer_module(&module, &prelude);
    assert!(
        result.is_ok(),
        "lambda com hint deve succeed: {:?}",
        result.err()
    );
}
