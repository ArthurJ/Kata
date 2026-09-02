//! Catálogo de delegações `refines` — mapeia tipo refined → interfaces delegadas,
//! com rastreio de origem (origin).
//!
//! Populado no resolution (Pass 0) a partir de `RefinesDecl`.
//! Consumido no inference (fallback no dispatch) para substituir args refined
//! pelo tipo base e retentar dispatch.
//!
//! Cada delegação é registrada com `origin` (módulo de origem). Lookups
//! não-qualificados resolvem a origin automaticamente quando há apenas uma;
//! quando há múltiplas origins (nome ambíguo), o caller deve usar
//! `*_with_origin` para desambiguar.
//!
//! `refines` não registra no InterfaceRegistry e não cria overloads no
//! DispatchTable. O mecanismo é fallback em `apply.rs` (D1 do PRD-refines).

use std::collections::{HashMap, HashSet};

use crate::ty::Ty;

/// Uma delegação `refines`: tipo refined delega uma interface ao seu tipo base.
#[derive(Debug, Clone, PartialEq)]
pub struct RefinesEntry {
    /// Origem da delegação (módulo onde foi declarada).
    pub origin: String,
    /// Nome do tipo refined (ex: `PositiveInt`).
    pub type_name: String,
    /// Tipo base resolvido (ex: `Ty::Prim(PrimTy::Int)`).
    pub base_ty: Ty,
    /// Nome da interface delegada (ex: `NUM`).
    pub interface_name: String,
}

/// Catálogo de delegações `refines` por tipo, com rastreio de origem.
#[derive(Debug, Clone, Default)]
pub struct RefinesRegistry {
    /// (origin, type_name) → lista de delegações.
    entries: HashMap<(String, String), Vec<RefinesEntry>>,
    /// type_name → conjunto de origins que definem refines para este tipo.
    origins: HashMap<String, HashSet<String>>,
    /// Nomes ambíguos (definidos em múltiplas origins).
    ambiguous: HashSet<String>,
}

impl RefinesRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Registro ──────────────────────────────────────────

    /// Registra uma delegação `refines` com origin.
    pub fn register(&mut self, entry: RefinesEntry) {
        let origin = entry.origin.clone();
        let type_name = entry.type_name.clone();
        let key = (origin.clone(), type_name.clone());
        self.entries.entry(key).or_default().push(entry);
        self.track_origin(&type_name, &origin);
    }

    /// Rastreia a origin e marca ambíguo se >1 origin.
    fn track_origin(&mut self, type_name: &str, origin: &str) {
        let origins = self.origins.entry(type_name.to_string()).or_default();
        origins.insert(origin.to_string());
        if origins.len() > 1 {
            self.ambiguous.insert(type_name.to_string());
        }
    }

    // ── Resolução de origin ───────────────────────────────

    /// Retorna true se o type_name é ambíguo.
    pub fn is_ambiguous(&self, type_name: &str) -> bool {
        self.ambiguous.contains(type_name)
    }

    /// Resolve a origin de um tipo não-qualificado.
    ///
    /// Prefere `"__local__"` quando há múltiplas origins (shadowing do
    /// usuário sobre o prelude). Retorna `None` se ambíguo sem
    /// `"__local__"` ou não existe.
    pub fn resolve_origin(&self, type_name: &str) -> Option<&str> {
        self.origins.get(type_name).and_then(|origins| {
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

    /// Busca todas as delegações de um tipo refined (não-qualificado).
    /// Retorna slice vazio se o tipo não tem `refines` ou é ambíguo.
    pub fn get(&self, type_name: &str) -> &[RefinesEntry] {
        let origin = match self.resolve_origin(type_name) {
            Some(o) => o,
            None => return &[],
        };
        let key = (origin.to_string(), type_name.to_string());
        self.entries.get(&key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// `get` com origin explícita.
    pub fn get_with_origin(&self, origin: &str, type_name: &str) -> &[RefinesEntry] {
        let key = (origin.to_string(), type_name.to_string());
        self.entries.get(&key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Verifica se um tipo tem delegações `refines`.
    pub fn has_refines(&self, type_name: &str) -> bool {
        !self.get(type_name).is_empty()
    }

    /// Lista os nomes das interfaces que um tipo refined delega.
    pub fn interfaces_of(&self, type_name: &str) -> Vec<&str> {
        self.get(type_name)
            .iter()
            .map(|e| e.interface_name.as_str())
            .collect()
    }

    /// Itera sobre todas as entradas de todos os tipos. Usado para validação
    /// post-merge em `infer_module`.
    pub fn all_entries(&self) -> impl Iterator<Item = &RefinesEntry> {
        self.entries.values().flat_map(|v| v.iter())
    }

    // ── Merge ─────────────────────────────────────────────

    /// Mescla outro RefinesRegistry neste.
    /// Entradas de origins diferentes coexistem; nomes com múltiplas
    /// origins são marcados como ambíguos.
    pub fn merge(&mut self, other: RefinesRegistry) {
        for ((origin, type_name), entries) in other.entries {
            let key = (origin.clone(), type_name.clone());
            self.entries.insert(key, entries);
            self.track_origin(&type_name, &origin);
        }
    }
}

#[cfg(test)]
#[path = "refines_registry_tests.rs"]
mod tests;
