//! Catálogo de structs com campos e offsets, com rastreio de origem (origin).
//!
//! Populado no resolution (Pass 0) a partir de `DataDecl` com campos não-vazios.
//! Consumido no inference para field access e ascription-construção.
//!
//! Cada struct é registrado com `origin` (módulo de origem: "core", "my_module", etc).
//! Lookups não-qualificados resolvem a origin automaticamente quando há apenas uma;
//! quando há múltiplas origins (nome ambíguo), `is_ambiguous` retorna true e
//! o caller deve usar `*_with_origin` para desambiguar.
//!
//! Análogo ao `EnumRegistry` — definido em `kata-core` para evitar dependência
//! circular, populado no resolution, consumido no inference.

use std::collections::{HashMap, HashSet};

use crate::ty::Ty;

/// Informação de um campo de struct.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldInfo {
    /// Nome do campo (ex: `nome`, `idade`).
    pub name: String,
    /// Tipo do campo.
    pub ty: Ty,
    /// Offset em bytes = field_index * 8.
    /// Todos os campos são words de 8 bytes — structs são blocos contíguos
    /// de `n * 8` bytes na arena.
    pub offset: u32,
}

/// Informação de um struct registrado.
#[derive(Debug, Clone, PartialEq)]
pub struct StructInfo {
    /// Nome do struct (ex: `Pessoa`).
    pub name: String,
    /// Campos em ordem de declaração.
    pub fields: Vec<FieldInfo>,
    /// Se este struct é um alias (newtype) de outro tipo.
    /// `Some("Float")` significa `alias Float as Altura`.
    /// `None` para structs nativos declarados com `data`.
    pub alias_of: Option<String>,
    /// Nomes das funções predicado no DispatchTable.
    /// `None` = struct normal. `Some(vec)` = tipo refinado.
    /// Cada nome é uma função `BaseTy => Boolean` sintetizada no resolution.
    pub predicates: Option<Vec<String>>,
}

impl StructInfo {
    /// Número de campos.
    #[allow(dead_code)] // usado apenas em testes
    pub(crate) fn num_fields(&self) -> usize {
        self.fields.len()
    }

    /// Tamanho em bytes = num_fields * 8.
    #[allow(dead_code)] // usado apenas em testes
    pub(crate) fn size_bytes(&self) -> u32 {
        self.num_fields() as u32 * 8
    }

    /// Busca um campo pelo nome. Retorna `(field_index, &FieldInfo)`.
    pub fn find_field(&self, name: &str) -> Option<(u32, &FieldInfo)> {
        self.fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == name)
            .map(|(i, f)| (i as u32, f))
    }

    /// Lista os tipos dos campos em ordem (para shape check de ascription-construção).
    #[allow(dead_code)] // usado apenas em testes
    pub(crate) fn field_types(&self) -> Vec<&Ty> {
        self.fields.iter().map(|f| &f.ty).collect()
    }
}

/// Catálogo de structs por nome, com rastreio de origem.
#[derive(Debug, Clone, Default)]
pub struct StructRegistry {
    /// (origin, struct_name) → StructInfo.
    structs: HashMap<(String, String), StructInfo>,
    /// struct_name → conjunto de origins que definem este struct.
    origins: HashMap<String, HashSet<String>>,
    /// Nomes ambíguos (definidos em múltiplas origins).
    ambiguous: HashSet<String>,
}

impl StructRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Registro ──────────────────────────────────────────

    /// Registra um struct com seus campos.
    /// Offset de cada campo = field_index * 8.
    pub fn register(&mut self, origin: &str, name: &str, fields: Vec<FieldInfo>) {
        self.register_with_alias(origin, name, fields, None);
    }

    /// Registra um struct (ou alias) com campos e info de alias.
    pub fn register_with_alias(
        &mut self,
        origin: &str,
        name: &str,
        fields: Vec<FieldInfo>,
        alias_of: Option<String>,
    ) {
        let key = (origin.to_string(), name.to_string());
        self.structs.insert(
            key,
            StructInfo {
                name: name.to_string(),
                fields,
                alias_of,
                predicates: None,
            },
        );
        self.track_origin(name, origin);
    }

    /// Registra um tipo refinado.
    /// `alias_of` é o tipo base, `predicates` são nomes de funções no DispatchTable.
    pub fn register_refined(
        &mut self,
        origin: &str,
        name: &str,
        alias_of: &str,
        predicates: Vec<String>,
    ) {
        let key = (origin.to_string(), name.to_string());
        self.structs.insert(
            key,
            StructInfo {
                name: name.to_string(),
                fields: Vec::new(),
                alias_of: Some(alias_of.to_string()),
                predicates: Some(predicates),
            },
        );
        self.track_origin(name, origin);
    }

    /// Rastreia a origin de um struct e marca ambíguo se >1 origin.
    fn track_origin(&mut self, name: &str, origin: &str) {
        let origins = self.origins.entry(name.to_string()).or_default();
        origins.insert(origin.to_string());
        if origins.len() > 1 {
            self.ambiguous.insert(name.to_string());
        }
    }

    // ── Resolução de origin ───────────────────────────────

    /// Retorna true se o struct_name é ambíguo (definido em múltiplas origins).
    pub fn is_ambiguous(&self, name: &str) -> bool {
        self.ambiguous.contains(name)
    }

    /// Retorna as origins que definem este struct.
    #[allow(dead_code)]
    pub fn origins_of(&self, name: &str) -> Vec<&str> {
        self.origins
            .get(name)
            .map(|s| s.iter().map(|o| o.as_str()).collect())
            .unwrap_or_default()
    }

    /// Resolve a origin de um struct não-qualificado.
    /// Resolve a origin de um struct não-qualificado.
    ///
    /// Prefere `"__local__"` quando há múltiplas origins (shadowing do
    /// usuário sobre o prelude). Retorna `None` se ambíguo sem
    /// `"__local__"` ou não existe.
    pub fn resolve_origin(&self, name: &str) -> Option<&str> {
        self.origins.get(name).and_then(|origins| {
            if origins.len() == 1 {
                origins.iter().next().map(|s| s.as_str())
            } else if origins.contains("__local__") {
                Some("__local__")
            } else {
                None
            }
        })
    }

    // ── Consulta ──────────────────────────────────────────

    /// Busca informações de um struct pelo nome (não-qualificado).
    /// Retorna `None` se o nome é ambíguo ou não existe.
    pub fn get(&self, name: &str) -> Option<&StructInfo> {
        let origin = self.resolve_origin(name)?;
        let key = (origin.to_string(), name.to_string());
        self.structs.get(&key)
    }

    /// `get` com origin explícita.
    pub fn get_with_origin(&self, origin: &str, name: &str) -> Option<&StructInfo> {
        let key = (origin.to_string(), name.to_string());
        self.structs.get(&key)
    }

    /// Verifica se um nome é um struct registrado.
    #[allow(dead_code)] // usado apenas em testes
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Lista os nomes de todos os structs registrados (não-ambíguos).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.origins.keys().map(|s| s.as_str())
    }

    // ── Merge ─────────────────────────────────────────────

    /// Mescla outro StructRegistry neste.
    /// Structs de origins diferentes coexistem; nomes com múltiplas origins
    /// são marcados como ambíguos. Structs da mesma origin são sobrescritos
    /// (re-registro no mesmo módulo).
    pub fn merge(&mut self, other: StructRegistry) {
        for ((origin, name), info) in other.structs {
            let key = (origin.clone(), name.clone());
            self.structs.insert(key, info);
            self.track_origin(&name, &origin);
        }
    }

    /// Filtra structs mantendo apenas aqueles cujo nome está no `closure`
    /// ou cuja origin é `core` (prelude). Usado por `filter_exports`.
    pub fn retain_by_closure(&mut self, closure: &std::collections::HashSet<String>) {
        self.structs.retain(|(_, name), _| {
            closure.contains(name) || {
                self.origins
                    .get(name)
                    .is_some_and(|origins| origins.contains("core"))
            }
        });
        // Reconstruir origins e ambiguous
        self.origins.clear();
        self.ambiguous.clear();
        for (origin, name) in self.structs.keys() {
            let origins = self.origins.entry(name.clone()).or_default();
            origins.insert(origin.clone());
            if origins.len() > 1 {
                self.ambiguous.insert(name.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, ty: Ty, offset: u32) -> FieldInfo {
        FieldInfo {
            name: name.into(),
            ty,
            offset,
        }
    }

    #[test]
    fn register_and_query() {
        let mut registry = StructRegistry::new();
        registry.register(
            "user",
            "Pessoa",
            vec![field("nome", Ty::text(), 0), field("idade", Ty::int(), 8)],
        );

        assert!(registry.contains("Pessoa"));
        assert!(!registry.contains("Inexistente"));

        let info = registry.get("Pessoa").unwrap();
        assert_eq!(info.num_fields(), 2);
        assert_eq!(info.size_bytes(), 16);
    }

    #[test]
    fn find_field_by_name() {
        let mut registry = StructRegistry::new();
        registry.register(
            "user",
            "Pessoa",
            vec![field("nome", Ty::text(), 0), field("idade", Ty::int(), 8)],
        );

        let info = registry.get("Pessoa").unwrap();
        let (idx, f) = info.find_field("idade").unwrap();
        assert_eq!(idx, 1);
        assert_eq!(f.ty, Ty::int());
        assert_eq!(f.offset, 8);
    }

    #[test]
    fn find_nonexistent_field_returns_none() {
        let mut registry = StructRegistry::new();
        registry.register("user", "Pessoa", vec![field("nome", Ty::text(), 0)]);

        let info = registry.get("Pessoa").unwrap();
        assert!(info.find_field("inexistente").is_none());
    }

    #[test]
    fn field_types_in_order() {
        let mut registry = StructRegistry::new();
        registry.register(
            "user",
            "Pessoa",
            vec![field("nome", Ty::text(), 0), field("idade", Ty::int(), 8)],
        );

        let info = registry.get("Pessoa").unwrap();
        let types = info.field_types();
        assert_eq!(types, vec![&Ty::text(), &Ty::int()]);
    }

    #[test]
    fn merge_two_registries() {
        let mut a = StructRegistry::new();
        a.register("user", "A", vec![field("x", Ty::int(), 0)]);

        let mut b = StructRegistry::new();
        b.register("user", "B", vec![field("y", Ty::text(), 0)]);

        a.merge(b);
        assert!(a.contains("A"));
        assert!(a.contains("B"));
    }

    #[test]
    fn empty_registry_returns_none() {
        let registry = StructRegistry::new();
        assert!(registry.get("Qualquer").is_none());
        assert!(!registry.contains("Qualquer"));
    }

    #[test]
    fn struct_with_zero_fields() {
        let mut registry = StructRegistry::new();
        registry.register("user", "Vazio", vec![]);

        let info = registry.get("Vazio").unwrap();
        assert_eq!(info.num_fields(), 0);
        assert_eq!(info.size_bytes(), 0);
    }

    // ── Testes de origin ──────────────────────────────────

    #[test]
    fn merge_different_origins_marks_ambiguous() {
        let mut a = StructRegistry::new();
        a.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

        let mut b = StructRegistry::new();
        b.register("user", "Pessoa", vec![field("nome", Ty::int(), 0)]);

        a.merge(b);
        assert!(a.is_ambiguous("Pessoa"));
        assert!(a.resolve_origin("Pessoa").is_none());
        assert!(a.get("Pessoa").is_none()); // ambíguo → None
    }

    #[test]
    fn merge_same_origin_overwrites() {
        let mut a = StructRegistry::new();
        a.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

        let mut b = StructRegistry::new();
        b.register("core", "Pessoa", vec![field("nome", Ty::int(), 0)]);

        a.merge(b);
        assert!(!a.is_ambiguous("Pessoa"));
        assert_eq!(a.resolve_origin("Pessoa"), Some("core"));
        let info = a.get("Pessoa").unwrap();
        assert_eq!(info.fields[0].ty, Ty::int()); // sobrescreveu
    }

    #[test]
    fn resolve_origin_single() {
        let mut registry = StructRegistry::new();
        registry.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

        assert_eq!(registry.resolve_origin("Pessoa"), Some("core"));
        assert!(!registry.is_ambiguous("Pessoa"));
    }

    #[test]
    fn resolve_origin_ambiguous_returns_none() {
        let mut a = StructRegistry::new();
        a.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

        let mut b = StructRegistry::new();
        b.register("user", "Pessoa", vec![field("nome", Ty::int(), 0)]);

        a.merge(b);
        assert!(a.resolve_origin("Pessoa").is_none());
    }

    #[test]
    fn get_with_origin_disambiguates() {
        let mut a = StructRegistry::new();
        a.register("core", "Pessoa", vec![field("nome", Ty::text(), 0)]);

        let mut b = StructRegistry::new();
        b.register("user", "Pessoa", vec![field("nome", Ty::int(), 0)]);

        a.merge(b);

        let core_info = a.get_with_origin("core", "Pessoa").unwrap();
        assert_eq!(core_info.fields[0].ty, Ty::text());

        let user_info = a.get_with_origin("user", "Pessoa").unwrap();
        assert_eq!(user_info.fields[0].ty, Ty::int());
    }
}
