//! Catálogo de interfaces e implementações, com rastreio de origem (origin).
//!
//! Populado no resolution (Pass 0+) a partir de `InterfaceDecl` e
//! `ImplementsDecl`. Consumido no inference para dispatch com `iface++`
//! no Score e para verificação de conformidade.
//!
//! Cada interface é registrada com `origin` (módulo de origem). Lookups
//! não-qualificados resolvem a origin automaticamente quando há apenas uma;
//! quando há múltiplas origins (nome ambíguo), o caller deve usar
//! `*_with_origin` para desambiguar.
//!
//! `impls` é diferente: múltiplos módulos podem implementar a mesma interface
//! para o mesmo tipo — impls de origins diferentes coexistem (extend). Só é
//! erro se o mesmo módulo (mesma origin) implementar a mesma interface para
//! o mesmo tipo duas vezes.
//!
//! Análogo ao `EnumRegistry`/`StructRegistry` — definido em `kata-core`
//! para evitar dependência circular, populado no resolution, consumido
//! no inference.

use std::collections::{HashMap, HashSet};

use crate::ty::Ty;

/// Interface registrada no InterfaceRegistry.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceInfo {
    /// Nome da interface (ex: `NUM`, `ORD`, `EQ`, `SHOW`).
    pub name: String,
    /// Super-traits (ex: `["ORD", "EQ"]` para `NUM implements ORD EQ`).
    pub supertraits: Vec<String>,
    /// Type params da interface (ex: `["A"]` para `ITERABLE(A)`).
    /// Vazio para interfaces não-parametrizadas.
    pub type_params: Vec<String>,
    /// Assinaturas obrigatórias da interface.
    pub signatures: Vec<InterfaceSignature>,
}

/// Assinatura dentro de interface — tipos já resolvidos (`Ty`).
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceSignature {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
}

/// Implementação registrada.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplEntry {
    /// Origem do impl (módulo onde foi declarado).
    pub origin: String,
    /// Tipo concreto que implementa (ex: `Int`, `Complex`).
    pub type_name: String,
    /// Type params do tipo (ex: `["A"]` para `List(A)`).
    /// Vazio para tipos não-genéricos.
    pub type_params: Vec<String>,
    /// Nome da interface implementada (ex: `NUM`).
    pub interface_name: String,
    /// Params da interface vinculados (ex: `["A"]` para `ITERABLE(A)`).
    /// Vazio para interfaces não-parametrizadas.
    pub iface_params: Vec<String>,
    /// Métodos do impl.
    pub methods: Vec<ImplMethodInfo>,
}

/// Método dentro de impl — tipos já resolvidos.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplMethodInfo {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    /// Símbolo FFI se o método é `@ffi`. `None` = corpo Kata.
    pub ffi_symbol: Option<String>,
}

/// Catálogo de interfaces e implementações, com rastreio de origem.
#[derive(Debug, Clone, Default)]
pub struct InterfaceRegistry {
    /// (origin, interface_name) → InterfaceInfo.
    interfaces: HashMap<(String, String), InterfaceInfo>,
    /// interface_name → conjunto de origins que definem esta interface.
    origins: HashMap<String, HashSet<String>>,
    /// Nomes ambíguos (definidos em múltiplas origins).
    ambiguous: HashSet<String>,
    /// Lista de implementações (coexistem entre módulos).
    impls: Vec<ImplEntry>,
}

impl InterfaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Registro ──────────────────────────────────────────

    /// Registra uma interface com origin. Valida supertraits e detecta ciclos.
    /// Retorna `Err(msg)` se a interface já existe na mesma origin ou se há
    /// ciclo de supertraits.
    pub fn register_interface(&mut self, origin: &str, info: InterfaceInfo) -> Result<(), String> {
        let name = info.name.clone();
        let key = (origin.to_string(), name.clone());

        if self.interfaces.contains_key(&key) {
            return Err(format!(
                "interface '{}' já declarada no módulo '{}'",
                name, origin
            ));
        }

        // Insere temporariamente para que o DFS possa ver a interface
        // sendo registrada como parte do grafo.
        self.interfaces.insert(key, info);
        self.track_origin(&name, origin);

        let mut visiting = HashSet::new();
        let result = self.check_cycle(origin, &name, &mut visiting);
        if result.is_err() {
            // Remove a interface recém-inserida em caso de ciclo.
            self.interfaces.remove(&(origin.to_string(), name.clone()));
            if let Some(origins) = self.origins.get_mut(&name) {
                origins.remove(origin);
            }
            // Limpa entradas vazias e reavalia ambiguous.
            if self.origins.get(&name).is_some_and(|s| s.is_empty()) {
                self.origins.remove(&name);
            }
            if self.origins.get(&name).is_none_or(|s| s.len() <= 1) {
                self.ambiguous.remove(&name);
            }
        }
        result
    }

    /// Registra uma implementação.
    ///
    /// Impls de origins diferentes coexistem (acumulam). Só é erro se o
    /// mesmo módulo (mesma origin) implementar a mesma interface para o
    /// mesmo tipo duas vezes.
    ///
    /// Se a interface não existe neste registry, ainda assim registra o
    /// ImplEntry — a interface pode estar no prelude (mergiado depois).
    pub fn register_impl(&mut self, entry: ImplEntry) -> Result<(), String> {
        // Detecta duplicação real: mesma origin + mesmo tipo + mesma interface.
        let duplicate = self.impls.iter().any(|e| {
            e.origin == entry.origin
                && e.type_name == entry.type_name
                && e.interface_name == entry.interface_name
        });
        if duplicate {
            return Err(format!(
                "impl duplicado: '{}' implements '{}' no módulo '{}'",
                entry.type_name, entry.interface_name, entry.origin
            ));
        }

        // Não validar se a interface existe aqui — o prelude (com as
        // interfaces) é mergeado depois do resolve. A validação acontece
        // em `validate_impls_after_merge`, chamada após o merge.
        self.impls.push(entry);
        Ok(())
    }

    /// Rastreia a origin de uma interface e marca ambíguo se >1 origin.
    fn track_origin(&mut self, name: &str, origin: &str) {
        let origins = self.origins.entry(name.to_string()).or_default();
        origins.insert(origin.to_string());
        if origins.len() > 1 {
            self.ambiguous.insert(name.to_string());
        }
    }

    // ── Resolução de origin ───────────────────────────────

    /// Retorna true se o interface_name é ambíguo (definido em múltiplas origins).
    pub fn is_ambiguous(&self, name: &str) -> bool {
        self.ambiguous.contains(name)
    }

    /// Retorna as origins que definem esta interface.
    #[allow(dead_code)]
    pub fn origins_of(&self, name: &str) -> Vec<&str> {
        self.origins
            .get(name)
            .map(|s| s.iter().map(|o| o.as_str()).collect())
            .unwrap_or_default()
    }

    /// Resolve a origin de uma interface não-qualificada.
    /// Resolve a origin de uma interface não-qualificada.
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

    // ── Consulta de interfaces ────────────────────────────

    /// Busca uma interface pelo nome (não-qualificado).
    /// Retorna `None` se o nome é ambíguo ou não existe.
    pub fn get_interface(&self, name: &str) -> Option<&InterfaceInfo> {
        let origin = self.resolve_origin(name)?;
        let key = (origin.to_string(), name.to_string());
        self.interfaces.get(&key)
    }

    /// `get_interface` com origin explícita.
    pub fn get_interface_with_origin(&self, origin: &str, name: &str) -> Option<&InterfaceInfo> {
        let key = (origin.to_string(), name.to_string());
        self.interfaces.get(&key)
    }

    /// Lista todas as interfaces registradas (todas as origins).
    pub fn all_interfaces(&self) -> impl Iterator<Item = &InterfaceInfo> {
        self.interfaces.values()
    }

    // ── Consulta de impls ─────────────────────────────────

    /// Lista todas as implementações de um tipo.
    pub fn get_impls_for_type(&self, type_name: &str) -> Vec<&ImplEntry> {
        self.impls
            .iter()
            .filter(|e| e.type_name == type_name)
            .collect()
    }

    /// Itera sobre todas as implementações (read-only).
    pub fn impls_view(&self) -> &[ImplEntry] {
        &self.impls
    }

    /// Lista todas as implementações de uma interface.
    pub fn get_impls_for_interface(&self, iface_name: &str) -> Vec<&ImplEntry> {
        self.impls
            .iter()
            .filter(|e| e.interface_name == iface_name)
            .collect()
    }

    // ── Verificação ───────────────────────────────────────

    /// Verifica se um tipo implementa uma interface (direto ou via supertrait).
    pub fn type_implements(&self, type_name: &str, iface_name: &str) -> bool {
        let direct = self
            .impls
            .iter()
            .any(|e| e.type_name == type_name && e.interface_name == iface_name);
        if direct {
            return true;
        }
        for e in self.impls.iter().filter(|e| e.type_name == type_name) {
            if self.iface_inherits(&e.interface_name, iface_name) {
                return true;
            }
        }
        false
    }

    /// Verifica se `iface` herda (direta ou indiretamente) de `target`.
    fn iface_inherits(&self, iface: &str, target: &str) -> bool {
        let Some(info) = self.get_interface(iface) else {
            return false;
        };
        for st in &info.supertraits {
            if st == target {
                return true;
            }
            if self.iface_inherits(st, target) {
                return true;
            }
        }
        false
    }

    /// Verifica se uma interface (ou suas supertraits) contém um método
    /// com o nome dado. Percorre a hierarquia de supertraits recursivamente.
    pub fn interface_has_method(&self, iface_name: &str, method_name: &str) -> bool {
        let Some(info) = self.get_interface(iface_name) else {
            return false;
        };
        if info.signatures.iter().any(|sig| sig.name == method_name) {
            return true;
        }
        info.supertraits
            .iter()
            .any(|st| self.interface_has_method(st, method_name))
    }

    // ── Ciclo ─────────────────────────────────────────────

    /// DFS para detectar ciclos de supertraits.
    /// `origin` é a origin da interface sendo verificada (para lookup direto).
    /// `visiting` rastreia interfaces no caminho atual.
    fn check_cycle(
        &self,
        origin: &str,
        iface: &str,
        visiting: &mut HashSet<String>,
    ) -> Result<(), String> {
        if visiting.contains(iface) {
            return Err(format!("ciclo de supertraits detectado: '{}'", iface));
        }
        // Tenta lookup com a origin específica primeiro, depois resolve_origin.
        let info = self
            .get_interface_with_origin(origin, iface)
            .or_else(|| self.get_interface(iface));
        let Some(info) = info else {
            return Ok(());
        };
        visiting.insert(iface.to_string());
        for st in &info.supertraits {
            self.check_cycle(origin, st, visiting)?;
        }
        visiting.remove(iface);
        Ok(())
    }

    // ── Merge ─────────────────────────────────────────────

    /// Mescla outro InterfaceRegistry neste.
    /// Interfaces de origins diferentes coexistem; nomes com múltiplas
    /// origins são marcados como ambíguos. Interfaces da mesma origin
    /// são sobrescritas. Impls são acumulados (coexistem).
    pub fn merge(&mut self, other: InterfaceRegistry) {
        for ((origin, name), info) in other.interfaces {
            let key = (origin.clone(), name.clone());
            self.interfaces.insert(key, info);
            self.track_origin(&name, &origin);
        }
        self.impls.extend(other.impls);
    }

    /// Valida que todo `ImplEntry` referencia uma interface que existe no
    /// registry. Deve ser chamado **após** o merge do prelude — antes disso,
    /// interfaces do prelude (NUM, SHOW, etc.) ainda não estão visíveis.
    ///
    /// Retorna a lista de warnings (interface não encontrada). Erros reais
    /// (typo, interface esquecida) aparecem aqui em vez de no `register_impl`.
    pub fn validate_impls_after_merge(&self) -> Vec<String> {
        self.impls
            .iter()
            .filter(|e| self.get_interface(&e.interface_name).is_none())
            .map(|e| {
                format!(
                    "interface '{}' não declarada (em implements para {}) — pode ser um typo ou falta de import",
                    e.interface_name, e.type_name
                )
            })
            .collect()
    }

    /// Filtra interfaces e impls mantendo apenas aqueles cujo nome está no
    /// `closure` ou cuja origin é `core` (prelude). Usado por `filter_exports`.
    pub fn retain_by_closure(&mut self, closure: &std::collections::HashSet<String>) {
        // Filtrar interfaces: manter se nome no closure ou origin é core
        self.interfaces.retain(|(_, name), _| {
            closure.contains(name) || {
                self.origins
                    .get(name)
                    .is_some_and(|origins| origins.contains("core"))
            }
        });
        // Filtrar impls: manter se type_name no closure ou origin é core
        self.impls
            .retain(|e| closure.contains(&e.type_name) || e.origin == "core");
        // Reconstruir origins e ambiguous
        self.origins.clear();
        self.ambiguous.clear();
        for (origin, name) in self.interfaces.keys() {
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

    fn iface(name: &str, supertraits: &[&str]) -> InterfaceInfo {
        InterfaceInfo {
            name: name.into(),
            supertraits: supertraits.iter().map(|s| s.to_string()).collect(),
            type_params: Vec::new(),
            signatures: Vec::new(),
        }
    }

    fn impl_entry(origin: &str, type_name: &str, iface_name: &str) -> ImplEntry {
        ImplEntry {
            origin: origin.into(),
            type_name: type_name.into(),
            type_params: Vec::new(),
            interface_name: iface_name.into(),
            iface_params: Vec::new(),
            methods: Vec::new(),
        }
    }

    #[test]
    fn register_and_query_interface() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("EQ", &[])).unwrap();
        reg.register_interface("core", iface("ORD", &["EQ"]))
            .unwrap();

        assert!(reg.get_interface("EQ").is_some());
        assert!(reg.get_interface("ORD").is_some());
        assert!(reg.get_interface("NUM").is_none());
    }

    #[test]
    fn duplicate_interface_same_origin_is_error() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("EQ", &[])).unwrap();
        let err = reg.register_interface("core", iface("EQ", &[]));
        assert!(err.is_err());
    }

    #[test]
    fn duplicate_interface_different_origin_coexists() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("EQ", &[])).unwrap();
        let result = reg.register_interface("user", iface("EQ", &[]));
        assert!(result.is_ok());
        assert!(reg.is_ambiguous("EQ"));
        assert!(reg.resolve_origin("EQ").is_none());
        assert!(reg.get_interface("EQ").is_none()); // ambíguo
        assert!(reg.get_interface_with_origin("core", "EQ").is_some());
        assert!(reg.get_interface_with_origin("user", "EQ").is_some());
    }

    #[test]
    fn cycle_detection() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("A", &["B"])).unwrap();
        let err = reg.register_interface("core", iface("B", &["A"]));
        assert!(err.is_err());
    }

    #[test]
    fn register_impl_accepts_unknown_interface() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("NUM", &["ORD"]))
            .unwrap();
        reg.register_impl(impl_entry("user", "Int", "NUM")).unwrap();

        let result = reg.register_impl(impl_entry("user", "Int", "SHOW"));
        assert!(result.is_ok());
        assert_eq!(reg.get_impls_for_interface("SHOW").len(), 1);
    }

    #[test]
    fn register_impl_rejects_duplicate_same_origin() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("NUM", &[])).unwrap();
        reg.register_impl(impl_entry("user", "Int", "NUM")).unwrap();
        let err = reg.register_impl(impl_entry("user", "Int", "NUM"));
        assert!(err.is_err());
    }

    #[test]
    fn register_impl_allows_same_impl_different_origin() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("NUM", &[])).unwrap();
        reg.register_impl(impl_entry("core", "Int", "NUM")).unwrap();
        let result = reg.register_impl(impl_entry("user", "Int", "NUM"));
        assert!(result.is_ok());
    }

    #[test]
    fn type_implements_direct() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("NUM", &["ORD"]))
            .unwrap();
        reg.register_interface("core", iface("ORD", &["EQ"]))
            .unwrap();
        reg.register_interface("core", iface("EQ", &[])).unwrap();
        reg.register_impl(impl_entry("user", "Int", "NUM")).unwrap();

        assert!(reg.type_implements("Int", "NUM"));
        assert!(reg.type_implements("Int", "ORD"));
        assert!(reg.type_implements("Int", "EQ"));
        assert!(!reg.type_implements("Int", "SHOW"));
        assert!(!reg.type_implements("Float", "NUM"));
    }

    #[test]
    fn get_impls_for_type_and_interface() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface("core", iface("NUM", &[])).unwrap();
        reg.register_interface("core", iface("SHOW", &[])).unwrap();
        reg.register_impl(impl_entry("user", "Int", "NUM")).unwrap();
        reg.register_impl(impl_entry("user", "Int", "SHOW"))
            .unwrap();
        reg.register_impl(impl_entry("user", "Float", "NUM"))
            .unwrap();

        assert_eq!(reg.get_impls_for_type("Int").len(), 2);
        assert_eq!(reg.get_impls_for_type("Float").len(), 1);
        assert_eq!(reg.get_impls_for_interface("NUM").len(), 2);
        assert_eq!(reg.get_impls_for_interface("SHOW").len(), 1);
    }

    #[test]
    fn merge_two_registries() {
        let mut a = InterfaceRegistry::new();
        a.register_interface("core", iface("EQ", &[])).unwrap();

        let mut b = InterfaceRegistry::new();
        b.register_interface("core", iface("NUM", &["ORD"]))
            .unwrap();
        b.register_impl(impl_entry("user", "Int", "NUM")).unwrap();

        a.merge(b);
        assert!(a.get_interface("EQ").is_some());
        assert!(a.get_interface("NUM").is_some());
        assert!(a.type_implements("Int", "NUM"));
    }

    #[test]
    fn merge_different_origins_marks_ambiguous() {
        let mut a = InterfaceRegistry::new();
        a.register_interface("core", iface("EQ", &[])).unwrap();

        let mut b = InterfaceRegistry::new();
        b.register_interface("user", iface("EQ", &[])).unwrap();

        a.merge(b);
        assert!(a.is_ambiguous("EQ"));
        assert!(a.resolve_origin("EQ").is_none());
        assert!(a.get_interface("EQ").is_none());
    }
}
