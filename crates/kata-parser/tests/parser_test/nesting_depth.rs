//! Testes de limite de aninhamento de expressão (A3b).
//!
//! O parser recursive descent sem limitador de profundidade causa stack
//! overflow em ~800 níveis de aninhamento. A correção adiciona um contador
//! `depth` no Parser e rejeita aninhamento além de MAX_EXPR_DEPTH (256)
//! com `FrontendError::NestingTooDeep` gracioso.
//!
//! Os testes rodam em thread com stack 32MB para matching com produção
//! (o compilador roda no main thread, 8MB default no Linux; testes usam
//! spawned threads com 2MB default, insuficiente para os ~7 frames Rust
//! por nível de aninhamento do parser em debug mode).

use kata_diagnostics::FrontendError;
use kata_lexer::lex;
use kata_parser::parse;

/// Roda uma closure em thread com stack 32MB.
fn run_with_big_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn parse_err(src: &str) -> FrontendError {
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap_err()
}

/// 256 níveis de aninhamento de lista deve ser aceito sem erro.
#[test]
fn nesting_at_limit_ok() {
    run_with_big_stack(|| {
        let depth = 256;
        let src = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let tokens = lex(&src).unwrap();
        let result = parse(tokens);
        assert!(
            result.is_ok(),
            "256 níveis de aninhamento deve ser aceito, got: {:?}",
            result.err()
        );
    });
}

/// 257 níveis de aninhamento de lista deve produzir NestingTooDeep gracioso.
#[test]
fn nesting_over_limit_rejected() {
    run_with_big_stack(|| {
        let depth = 257;
        let src = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let err = parse_err(&src);
        assert!(
            matches!(err, FrontendError::NestingTooDeep { limit: 256, .. }),
            "257 níveis deve dar NestingTooDeep, got: {err:?}"
        );
    });
}

/// Aninhamento via parênteses também deve ser limitado.
#[test]
fn nesting_parens_over_limit_rejected() {
    run_with_big_stack(|| {
        let depth = 257;
        let src = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
        let err = parse_err(&src);
        assert!(
            matches!(err, FrontendError::NestingTooDeep { limit: 256, .. }),
            "257 níveis de parênteses deve dar NestingTooDeep, got: {err:?}"
        );
    });
}

/// 256 níveis via parênteses deve ser aceito.
#[test]
fn nesting_parens_at_limit_ok() {
    run_with_big_stack(|| {
        let depth = 256;
        let src = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
        let tokens = lex(&src).unwrap();
        let result = parse(tokens);
        assert!(
            result.is_ok(),
            "256 níveis de parênteses deve ser aceito, got: {:?}",
            result.err()
        );
    });
}

/// Aninhamento moderado (100 níveis) deve funcionar normalmente.
#[test]
fn nesting_moderate_ok() {
    run_with_big_stack(|| {
        let src = format!("{}1{}", "[".repeat(100), "]".repeat(100));
        let tokens = lex(&src).unwrap();
        let result = parse(tokens);
        assert!(
            result.is_ok(),
            "100 níveis deve ser aceito, got: {:?}",
            result.err()
        );
    });
}
