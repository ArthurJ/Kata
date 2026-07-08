use kata_ast::Span;
use kata_diagnostics::{FrontendError, MiddleError};

#[test]
fn frontend_error_unexpected_token_display() {
    let err = FrontendError::UnexpectedToken {
        expected: "IntLit".into(),
        found: "Ident".into(),
        span: Span::new(0, 1, 1, 3).into(),
    };
    assert_eq!(
        err.to_string(),
        "token inesperado: esperado `IntLit`, encontrado `Ident`"
    );
}

#[test]
fn frontend_error_invalid_char_display() {
    let err = FrontendError::InvalidChar {
        char: "@".into(),
        span: Span::new(5, 1, 6, 1).into(),
    };
    assert_eq!(err.to_string(), "caractere inválido: `@`");
}

#[test]
fn frontend_error_unterminated_string_display() {
    let err = FrontendError::UnterminatedString {
        span: Span::new(0, 1, 1, 10).into(),
    };
    assert_eq!(err.to_string(), "string não terminada");
}

#[test]
fn middle_error_type_mismatch_display() {
    let err = MiddleError::TypeMismatch {
        expected: "Int".into(),
        found: "Float".into(),
        span: Span::new(0, 1, 1, 3).into(),
    };
    assert_eq!(
        err.to_string(),
        "tipo incompatível: esperado `Int`, encontrado `Float`"
    );
}

#[test]
fn middle_error_unbound_name_display() {
    let err = MiddleError::UnboundName {
        name: "foo".into(),
        span: Span::new(0, 1, 1, 3).into(),
    };
    assert_eq!(err.to_string(), "nome `foo` não está no escopo");
}

#[test]
fn middle_error_ambiguous_dispatch_display() {
    let err = MiddleError::AmbiguousDispatch {
        name: "+".into(),
        span: Span::new(0, 1, 1, 1).into(),
    };
    assert_eq!(
        err.to_string(),
        "dispatch ambíguo: múltiplas sobrecargas compatíveis para `+`"
    );
}

#[test]
fn miette_span_from_span() {
    let span = Span::new(10, 2, 5, 3);
    let ms: kata_diagnostics::MietteSpan = span.into();
    assert_eq!(ms.0, span);
}

#[test]
fn miette_span_into_source_span() {
    let span = Span::new(10, 2, 5, 3);
    let ms: kata_diagnostics::MietteSpan = span.into();
    let ss: miette::SourceSpan = ms.into();
    assert_eq!(ss.offset(), 10);
    assert_eq!(ss.len(), 3);
}
