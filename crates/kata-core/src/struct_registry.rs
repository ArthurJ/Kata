//! Catálogo de structs com campos e offsets.
//!
//! Populado no resolution (Pass 0) a partir de `DataDecl` com campos não-vazios.
//! Consumido no inference para field access e ascription-construção.
//!
//! Análogo ao `EnumRegistry` — definido em `kata-core` para evitar dependência
//! circular, populado no resolution, consumido no inference.

use std::collections::HashMap;

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

/// Catálogo de structs por nome.
#[derive(Debug, Clone, Default)]
pub struct StructRegistry {
    /// struct_name → StructInfo.
    structs: HashMap<String, StructInfo>,
}

impl StructRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra um struct com seus campos.
    /// Offset de cada campo = field_index * 8.
    pub fn register(&mut self, name: &str, fields: Vec<FieldInfo>) {
        self.register_with_alias(name, fields, None);
    }

    /// Registra um struct (ou alias) com campos e info de alias.
    pub fn register_with_alias(
        &mut self,
        name: &str,
        fields: Vec<FieldInfo>,
        alias_of: Option<String>,
    ) {
        self.structs.insert(
            name.to_string(),
            StructInfo {
                name: name.to_string(),
                fields,
                alias_of,
                predicates: None,
            },
        );
    }

    /// Registra um tipo refinado.
    /// `alias_of` é o tipo base, `predicates` são nomes de funções no DispatchTable.
    pub fn register_refined(&mut self, name: &str, alias_of: &str, predicates: Vec<String>) {
        self.structs.insert(
            name.to_string(),
            StructInfo {
                name: name.to_string(),
                fields: Vec::new(),
                alias_of: Some(alias_of.to_string()),
                predicates: Some(predicates),
            },
        );
    }

    /// Registra um struct com `StructInfo` pronto.
    #[allow(dead_code)] // zero callers — scaffolding para registro direto de StructInfo
    pub(crate) fn register_info(&mut self, info: StructInfo) {
        self.structs.insert(info.name.clone(), info);
    }

    /// Busca informações de um struct pelo nome.
    pub fn get(&self, name: &str) -> Option<&StructInfo> {
        self.structs.get(name)
    }

    /// Verifica se um nome é um struct registrado.
    #[allow(dead_code)] // usado apenas em testes
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.structs.contains_key(name)
    }

    /// Lista os nomes de todos os structs registrados.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.structs.keys().map(|s| s.as_str())
    }

    /// Mescla outro StructRegistry neste. Structs do outro sobrescrevem
    /// structs com o mesmo nome. Usado para combinar prelude + user module.
    pub fn merge(&mut self, other: StructRegistry) {
        for (name, info) in other.structs {
            self.structs.insert(name, info);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_query() {
        let mut registry = StructRegistry::new();
        registry.register(
            "Pessoa",
            vec![
                FieldInfo {
                    name: "nome".into(),
                    ty: Ty::text(),
                    offset: 0,
                },
                FieldInfo {
                    name: "idade".into(),
                    ty: Ty::int(),
                    offset: 8,
                },
            ],
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
            "Pessoa",
            vec![
                FieldInfo {
                    name: "nome".into(),
                    ty: Ty::text(),
                    offset: 0,
                },
                FieldInfo {
                    name: "idade".into(),
                    ty: Ty::int(),
                    offset: 8,
                },
            ],
        );

        let info = registry.get("Pessoa").unwrap();
        let (idx, field) = info.find_field("idade").unwrap();
        assert_eq!(idx, 1);
        assert_eq!(field.ty, Ty::int());
        assert_eq!(field.offset, 8);
    }

    #[test]
    fn find_nonexistent_field_returns_none() {
        let mut registry = StructRegistry::new();
        registry.register(
            "Pessoa",
            vec![FieldInfo {
                name: "nome".into(),
                ty: Ty::text(),
                offset: 0,
            }],
        );

        let info = registry.get("Pessoa").unwrap();
        assert!(info.find_field("inexistente").is_none());
    }

    #[test]
    fn field_types_in_order() {
        let mut registry = StructRegistry::new();
        registry.register(
            "Pessoa",
            vec![
                FieldInfo {
                    name: "nome".into(),
                    ty: Ty::text(),
                    offset: 0,
                },
                FieldInfo {
                    name: "idade".into(),
                    ty: Ty::int(),
                    offset: 8,
                },
            ],
        );

        let info = registry.get("Pessoa").unwrap();
        let types = info.field_types();
        assert_eq!(types, vec![&Ty::text(), &Ty::int()]);
    }

    #[test]
    fn merge_two_registries() {
        let mut a = StructRegistry::new();
        a.register(
            "A",
            vec![FieldInfo {
                name: "x".into(),
                ty: Ty::int(),
                offset: 0,
            }],
        );

        let mut b = StructRegistry::new();
        b.register(
            "B",
            vec![FieldInfo {
                name: "y".into(),
                ty: Ty::text(),
                offset: 0,
            }],
        );

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
        registry.register("Vazio", vec![]);

        let info = registry.get("Vazio").unwrap();
        assert_eq!(info.num_fields(), 0);
        assert_eq!(info.size_bytes(), 0);
    }
}
