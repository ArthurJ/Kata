//! Catálogo de variantes por enum.
//!
//! Populado no resolution (Pass 0) a partir de `EnumDecl`.
//! Consumido no inference para resolver patterns desqualificados
//! (`True` → `Variant { enum_name: "Boolean", variant: "True" }`)
//! e verificar exaustividade de match em `Sum`.

use std::collections::HashMap;

/// Catálogo de variantes por enum.
#[derive(Debug, Clone, Default)]
pub struct EnumRegistry {
    /// enum_name → lista de nomes de variantes (em ordem de declaração).
    variants: HashMap<String, Vec<String>>,
}

impl EnumRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra um enum com suas variantes.
    pub fn register(&mut self, enum_name: &str, variants: Vec<String>) {
        self.variants.insert(enum_name.to_string(), variants);
    }

    /// Verifica se um nome é variante de um enum.
    pub fn is_variant(&self, enum_name: &str, variant: &str) -> bool {
        self.variants
            .get(enum_name)
            .is_some_and(|vs| vs.iter().any(|v| v == variant))
    }

    /// Lista as variantes de um enum (para verificação de exaustividade).
    pub fn variants_of(&self, enum_name: &str) -> &[String] {
        self.variants
            .get(enum_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Busca o enum ao qual uma variante pertence.
    /// Retorna o nome do enum se `variant_name` for uma variante conhecida
    /// de algum enum. Usado para resolver patterns desqualificados.
    pub fn find_enum_of_variant(&self, variant_name: &str) -> Option<&str> {
        self.variants
            .iter()
            .find(|(_, vs)| vs.iter().any(|v| v == variant_name))
            .map(|(enum_name, _)| enum_name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_query() {
        let mut registry = EnumRegistry::new();
        registry.register("Boolean", vec!["True".into(), "False".into()]);

        assert!(registry.is_variant("Boolean", "True"));
        assert!(registry.is_variant("Boolean", "False"));
        assert!(!registry.is_variant("Boolean", "Maybe"));

        let variants = registry.variants_of("Boolean");
        assert_eq!(variants, &["True".to_string(), "False".to_string()]);
    }

    #[test]
    fn find_enum_of_variant() {
        let mut registry = EnumRegistry::new();
        registry.register("Boolean", vec!["True".into(), "False".into()]);

        assert_eq!(registry.find_enum_of_variant("True"), Some("Boolean"));
        assert_eq!(registry.find_enum_of_variant("False"), Some("Boolean"));
        assert_eq!(registry.find_enum_of_variant("Maybe"), None);
    }

    #[test]
    fn unknown_enum_returns_empty() {
        let registry = EnumRegistry::new();
        assert!(registry.variants_of("NonExistent").is_empty());
        assert!(!registry.is_variant("NonExistent", "Anything"));
    }
}
