//! Catálogo de interfaces e implementações.
//!
//! Populado no resolution (Pass 0+) a partir de `InterfaceDecl` e
//! `ImplementsDecl`. Consumido no inference para dispatch com `iface++`
//! no Score e para verificação de conformidade.
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

/// Catálogo de interfaces e implementações.
#[derive(Debug, Clone, Default)]
pub struct InterfaceRegistry {
    /// interface_name → InterfaceInfo.
    interfaces: HashMap<String, InterfaceInfo>,
    /// Lista de implementações.
    impls: Vec<ImplEntry>,
}

impl InterfaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra uma interface. Valida supertraits e detecta ciclos.
    /// Retorna `Err(msg)` se a interface já existe ou se há ciclo de supertraits.
    pub fn register_interface(&mut self, info: InterfaceInfo) -> Result<(), String> {
        if self.interfaces.contains_key(&info.name) {
            return Err(format!("interface '{}' já declarada", info.name));
        }
        // Detecta ciclo de supertraits antes de inserir.
        // Insere temporariamente para que o DFS possa ver a interface
        // sendo registrada como parte do grafo (necessário para detectar
        // ciclos que passam pela nova interface).
        let name = info.name.clone();
        self.interfaces.insert(name.clone(), info);
        let mut visiting = HashSet::new();
        let result = self.check_cycle(&name, &mut visiting);
        if result.is_err() {
            self.interfaces.remove(&name);
        }
        result
    }

    /// DFS para detectar ciclos de supertraits.
    /// `visiting` rastreia interfaces no caminho atual.
    fn check_cycle(&self, iface: &str, visiting: &mut HashSet<String>) -> Result<(), String> {
        if visiting.contains(iface) {
            return Err(format!("ciclo de supertraits detectado: '{}'", iface));
        }
        let Some(info) = self.interfaces.get(iface) else {
            // Supertrait não registrada — pode ser declarada depois.
            return Ok(());
        };
        visiting.insert(iface.to_string());
        for st in &info.supertraits {
            self.check_cycle(st, visiting)?;
        }
        visiting.remove(iface);
        Ok(())
    }

    /// Registra uma implementação.
    ///
    /// Se a interface não existe neste registry, ainda assim registra o
    /// ImplEntry — a interface pode estar no prelude (mergiado depois).
    /// A validação final acontece no inference, onde o InterfaceRegistry
    /// completo (prelude + user) está disponível.
    pub fn register_impl(&mut self, entry: ImplEntry) -> Result<(), String> {
        if !self.interfaces.contains_key(&entry.interface_name) {
            // Não é erro — a interface pode estar no prelude.
            // Apenas loga warning e registra mesmo assim.
            eprintln!(
                "[resolution] warning: interface '{}' não declarada neste módulo (em implements para {}) — pode estar no prelude",
                entry.interface_name, entry.type_name
            );
        }
        self.impls.push(entry);
        Ok(())
    }

    /// Busca uma interface pelo nome.
    pub fn get_interface(&self, name: &str) -> Option<&InterfaceInfo> {
        self.interfaces.get(name)
    }

    /// Lista todas as implementações de um tipo.
    pub fn get_impls_for_type(&self, type_name: &str) -> Vec<&ImplEntry> {
        self.impls
            .iter()
            .filter(|e| e.type_name == type_name)
            .collect()
    }

    /// Lista todas as implementações de uma interface.
    pub fn get_impls_for_interface(&self, iface_name: &str) -> Vec<&ImplEntry> {
        self.impls
            .iter()
            .filter(|e| e.interface_name == iface_name)
            .collect()
    }

    /// Verifica se um tipo implementa uma interface (direto ou via supertrait).
    ///
    /// `type_implements("Int", "NUM")` → true se há `Int implements NUM`.
    /// Também verifica supertraits: se `Int` implementa `NUM` e `NUM : ORD`,
    /// então `type_implements("Int", "ORD")` → true.
    pub fn type_implements(&self, type_name: &str, iface_name: &str) -> bool {
        // Busca direta: existe impl deste tipo para esta interface?
        let direct = self
            .impls
            .iter()
            .any(|e| e.type_name == type_name && e.interface_name == iface_name);
        if direct {
            return true;
        }
        // Busca indireta via supertraits: se o tipo implementa uma interface
        // que herda de `iface_name`, então o tipo também implementa `iface_name`.
        // Ex: impl NUM for Int + NUM : ORD → type_implements("Int", "ORD") = true.
        for e in self.impls.iter().filter(|e| e.type_name == type_name) {
            if self.iface_inherits(&e.interface_name, iface_name) {
                return true;
            }
        }
        false
    }

    /// Verifica se `iface` herda (direta ou indiretamente) de `target`.
    fn iface_inherits(&self, iface: &str, target: &str) -> bool {
        let Some(info) = self.interfaces.get(iface) else {
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

    /// Lista todas as interfaces registradas (para iteração).
    /// Usado pelo dispatch de `Ty::Var` em funções genéricas sintetizadas.
    pub fn all_interfaces(&self) -> impl Iterator<Item = &InterfaceInfo> {
        self.interfaces.values()
    }

    /// Mescla outro InterfaceRegistry neste. Interfaces e impls do outro
    /// são adicionados. Conflitos de nome de interface: o outro sobrescreve.
    /// Usado para combinar prelude + user module.
    pub fn merge(&mut self, other: InterfaceRegistry) {
        for (name, info) in other.interfaces {
            self.interfaces.insert(name, info);
        }
        self.impls.extend(other.impls);
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

    fn impl_entry(type_name: &str, iface_name: &str) -> ImplEntry {
        ImplEntry {
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
        reg.register_interface(iface("EQ", &[])).unwrap();
        reg.register_interface(iface("ORD", &["EQ"])).unwrap();

        assert!(reg.get_interface("EQ").is_some());
        assert!(reg.get_interface("ORD").is_some());
        assert!(reg.get_interface("NUM").is_none());
    }

    #[test]
    fn duplicate_interface_is_error() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface(iface("EQ", &[])).unwrap();
        let err = reg.register_interface(iface("EQ", &[]));
        assert!(err.is_err());
    }

    #[test]
    fn cycle_detection() {
        let mut reg = InterfaceRegistry::new();
        // A : B, B : A → ciclo
        reg.register_interface(iface("A", &["B"])).unwrap();
        let err = reg.register_interface(iface("B", &["A"]));
        assert!(err.is_err());
    }

    #[test]
    fn register_impl_accepts_unknown_interface() {
        // register_impl aceita implementações mesmo para interfaces não
        // declaradas neste registry — a interface pode estar no prelude.
        let mut reg = InterfaceRegistry::new();
        reg.register_interface(iface("NUM", &["ORD"])).unwrap();
        reg.register_impl(impl_entry("Int", "NUM")).unwrap();

        // Interface inexistente → ainda registra (pode estar no prelude)
        let result = reg.register_impl(impl_entry("Int", "SHOW"));
        assert!(result.is_ok());
        // O impl foi registrado
        assert_eq!(reg.get_impls_for_interface("SHOW").len(), 1);
    }

    #[test]
    fn type_implements_direct() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface(iface("NUM", &["ORD"])).unwrap();
        reg.register_interface(iface("ORD", &["EQ"])).unwrap();
        reg.register_interface(iface("EQ", &[])).unwrap();
        reg.register_impl(impl_entry("Int", "NUM")).unwrap();

        assert!(reg.type_implements("Int", "NUM"));
        // Via supertrait: NUM : ORD → Int implementa ORD
        assert!(reg.type_implements("Int", "ORD"));
        // Via supertrait em cadeia: NUM : ORD : EQ → Int implementa EQ
        assert!(reg.type_implements("Int", "EQ"));
        // Não implementa
        assert!(!reg.type_implements("Int", "SHOW"));
        assert!(!reg.type_implements("Float", "NUM"));
    }

    #[test]
    fn get_impls_for_type_and_interface() {
        let mut reg = InterfaceRegistry::new();
        reg.register_interface(iface("NUM", &[])).unwrap();
        reg.register_interface(iface("SHOW", &[])).unwrap();
        reg.register_impl(impl_entry("Int", "NUM")).unwrap();
        reg.register_impl(impl_entry("Int", "SHOW")).unwrap();
        reg.register_impl(impl_entry("Float", "NUM")).unwrap();

        assert_eq!(reg.get_impls_for_type("Int").len(), 2);
        assert_eq!(reg.get_impls_for_type("Float").len(), 1);
        assert_eq!(reg.get_impls_for_interface("NUM").len(), 2);
        assert_eq!(reg.get_impls_for_interface("SHOW").len(), 1);
    }

    #[test]
    fn merge_two_registries() {
        let mut a = InterfaceRegistry::new();
        a.register_interface(iface("EQ", &[])).unwrap();

        let mut b = InterfaceRegistry::new();
        b.register_interface(iface("NUM", &["ORD"])).unwrap();
        b.register_impl(impl_entry("Int", "NUM")).unwrap();

        a.merge(b);
        assert!(a.get_interface("EQ").is_some());
        assert!(a.get_interface("NUM").is_some());
        assert!(a.type_implements("Int", "NUM"));
    }
}
