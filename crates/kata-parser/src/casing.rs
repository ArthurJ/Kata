//! Validação de casing — `snake_case`, `PascalCase`, `ALL_CAPS`.
//!
//! Helper compartilhado chamado em cada site de parse onde um nome é
//! extraído. Garante que tipos, enums, funções, actions, interfaces,
//! variáveis e demais nomes sigam a convenção de capitalização do
//! projeto. A violação constitui erro fatal de compilação.

use kata_ast::Span;
use kata_diagnostics::{FrontendError, MietteSpan};

/// Padrão de casing esperado para um nome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CasingPattern {
    /// Primeiro char uppercase, sem `_` — ex: `Pessoa`, `Boolean`, `True`.
    PascalCase,
    /// Primeiro char lowercase, sem `_` no início — ex: `soma`, `minha_var`.
    SnakeCase,
    /// Tudo uppercase, `_` permitido — ex: `NUM`, `SHOW`, `ALL_CAPS`.
    AllCaps,
}

impl CasingPattern {
    fn label(self) -> &'static str {
        match self {
            CasingPattern::PascalCase => "PascalCase",
            CasingPattern::SnakeCase => "snake_case",
            CasingPattern::AllCaps => "ALL_CAPS",
        }
    }
}

/// Detecta o casing real de `name` para a mensagem de erro.
fn detect_casing(name: &str) -> &'static str {
    if name.is_empty() {
        return "vazio";
    }
    let first = name.chars().next().unwrap();
    if first == '_' || first.is_lowercase() {
        "snake_case"
    } else if first.is_uppercase() {
        if name.contains('_') {
            "ALL_CAPS"
        } else {
            "PascalCase"
        }
    } else {
        "desconhecido"
    }
}

/// Verifica se `name` é PascalCase: primeiro char uppercase, sem `_`.
fn is_pascal_case(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let first = name.chars().next().unwrap();
    first.is_uppercase() && !name.contains('_')
}

/// Verifica se `name` é snake_case: primeiro char lowercase ou `_`.
pub(crate) fn is_snake_case(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let first = name.chars().next().unwrap();
    first.is_lowercase() || first == '_'
}

/// Verifica se `name` é ALL_CAPS: todos uppercase (ou `_`), contém pelo
/// menos uma letra uppercase.
fn is_all_caps(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let has_uppercase = name.chars().any(|c| c.is_uppercase());
    let all_valid = name
        .chars()
        .all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '_');
    has_uppercase && all_valid
}

/// Valida `name` contra `expected`. Retorna `Ok(())` se conforme,
/// `Err(FrontendError::InvalidCasing)` caso contrário.
pub(crate) fn validate_casing(
    name: &str,
    expected: CasingPattern,
    span: Span,
) -> Result<(), FrontendError> {
    let ok = match expected {
        CasingPattern::PascalCase => is_pascal_case(name),
        CasingPattern::SnakeCase => is_snake_case(name),
        CasingPattern::AllCaps => is_all_caps(name),
    };
    if ok {
        Ok(())
    } else {
        Err(FrontendError::InvalidCasing {
            name: name.to_string(),
            expected_casing: expected.label().to_string(),
            found_casing: detect_casing(name).to_string(),
            span: MietteSpan::from(span),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_valid() {
        assert!(is_pascal_case("Pessoa"));
        assert!(is_pascal_case("Boolean"));
        assert!(is_pascal_case("True"));
        assert!(is_pascal_case("Int"));
        assert!(is_pascal_case("PositiveInt"));
    }

    #[test]
    fn pascal_case_invalid() {
        assert!(!is_pascal_case("pessoa"));
        assert!(!is_pascal_case("minha_enum"));
        assert!(!is_pascal_case("snake_case"));
        assert!(!is_pascal_case(""));
    }

    #[test]
    fn snake_case_valid() {
        assert!(is_snake_case("soma"));
        assert!(is_snake_case("minha_var"));
        assert!(is_snake_case("x"));
        assert!(is_snake_case("main"));
    }

    #[test]
    fn snake_case_invalid() {
        assert!(!is_snake_case("Soma"));
        assert!(!is_snake_case("MinhaVar"));
        assert!(!is_snake_case(""));
    }

    #[test]
    fn all_caps_valid() {
        assert!(is_all_caps("NUM"));
        assert!(is_all_caps("SHOW"));
        assert!(is_all_caps("ALL_CAPS"));
        assert!(is_all_caps("EQ"));
    }

    #[test]
    fn all_caps_invalid() {
        assert!(!is_all_caps("num"));
        assert!(!is_all_caps("Num"));
        assert!(!is_all_caps("NumCaps"));
        assert!(!is_all_caps(""));
    }
}
