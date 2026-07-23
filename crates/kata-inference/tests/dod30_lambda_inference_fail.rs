//! DoD 30 — LambdaInferenceFail.
//!
//! Quando nenhum mecanismo (partial dispatch, ascription de hole, hint top-down)
//! fornece o tipo de um parâmetro de lambda, o typeck produz `LambdaInferenceFail`
//! em vez de criar InferVar e deixar o dispatch falhar com `NoOverload` opaco.
//!
//! Caso base: `lambda x: + x 1` sem contexto de tipo — `x` é InferVar, `+` não
//! despacha, mas o erro deve ser `LambdaInferenceFail` (não `NoOverload`).

use kata_diagnostics::MiddleError;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::load_prelude;

fn infer_src_err(src: &str) -> MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
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

/// `lambda x y: + x y` — dois params sem contexto. Partial dispatch falha
/// (ambos são InferVar). Deve produzir LambdaInferenceFail.
#[test]
fn two_param_lambda_without_context_fails() {
    let err = infer_src_err("lambda x y: + x y");
    assert!(
        matches!(err, MiddleError::LambdaInferenceFail { .. }),
        "esperava LambdaInferenceFail, got {err:?}"
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

/// `lambda x y: + x y` sem contexto — partial dispatch tenta `+` com [?, ?]
/// mas ambos args são holes. `+` tem overloads [Int, Int] e [Float, Float] —
/// sem nenhum arg concreto para restringir, múltiplas overloads casam (ambíguo).
/// Deve produzir LambdaInferenceFail COM detail mencionando `+` e ambiguidade.
#[test]
fn lambda_inference_fail_has_detail() {
    let err = infer_src_err("lambda x y: + x y");
    match err {
        MiddleError::LambdaInferenceFail { detail, .. } => {
            let detail = detail.expect(
                "lambda com partial dispatch aplicável deve ter detail com contexto de falha"
            );
            assert!(
                detail.contains("+"),
                "detail deve mencionar a função tentada: {detail}"
            );
            assert!(
                detail.contains("amb") || detail.contains("Amb"),
                "detail deve mencionar ambiguidade: {detail}"
            );
        }
        other => panic!("esperava LambdaInferenceFail, got {other:?}"),
    }
}

/// `(lambda x: + x 1)::(Int -> Int)` — COM hint top-down, deve succeed.
/// Este teste confirma que o hint previne o LambdaInferenceFail.
#[test]
fn lambda_with_hint_does_not_fail() {
    let tokens = lex("(lambda x: + x 1)::(Int -> Int)").unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let result = infer_module(&module, &prelude);
    assert!(
        result.is_ok(),
        "lambda com hint deve succeed: {:?}",
        result.err()
    );
}
