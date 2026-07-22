//! Catálogo de delegações `refines` — mapeia tipo refined → interfaces delegadas.
//!
//! Populado no resolution (Pass 0) a partir de `RefinesDecl`.
//! Consumido no inference (fallback no dispatch) para substituir args refined
//! pelo tipo base e retentar dispatch.
//!
//! `refines` não registra no InterfaceRegistry e não cria overloads no
//! DispatchTable. O mecanismo é fallback em `apply.rs` (D1 do PRD-refines).

use std::collections::HashMap;

use crate::ty::Ty;

/// Uma delegação `refines`: tipo refined delega uma interface ao seu tipo base.
#[derive(Debug, Clone, PartialEq)]
pub struct RefinesEntry {
    /// Nome do tipo refined (ex: `PositiveInt`).
    pub type_name: String,
    /// Tipo base resolvido (ex: `Ty::Prim(PrimTy::Int)`).
    pub base_ty: Ty,
    /// Nome da interface delegada (ex: `NUM`).
    pub interface_name: String,
}

/// Catálogo de delegações `refines` por tipo.
#[derive(Debug, Clone, Default)]
pub struct RefinesRegistry {
    /// type_name → lista de delegações (um tipo pode refinar múltiplas interfaces).
    entries: HashMap<String, Vec<RefinesEntry>>,
}

impl RefinesRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra uma delegação `refines`.
    pub fn register(&mut self, entry: RefinesEntry) {
        self.entries
            .entry(entry.type_name.clone())
            .or_default()
            .push(entry);
    }

    /// Busca todas as delegações de um tipo refined.
    /// Retorna slice vazio se o tipo não tem `refines`.
    pub fn get(&self, type_name: &str) -> &[RefinesEntry] {
        self.entries
            .get(type_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Verifica se um tipo tem delegações `refines`.
    pub fn has_refines(&self, type_name: &str) -> bool {
        self.entries.contains_key(type_name)
    }

    /// Lista os nomes das interfaces que um tipo refined delega.
    pub fn interfaces_of(&self, type_name: &str) -> Vec<&str> {
        self.entries
            .get(type_name)
            .map(|v| v.iter().map(|e| e.interface_name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Mescla outro RefinesRegistry neste. Entradas do outro sobrescrevem
    /// para o mesmo tipo. Usado para combinar prelude + user module.
    pub fn merge(&mut self, other: RefinesRegistry) {
        for (name, entries) in other.entries {
            self.entries.insert(name, entries);
        }
    }
}