use std::collections::HashMap;
use crate::parser::ast::TypeRef;

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInfo {
    pub name: String,
    pub arity: usize,
    pub type_info: TypeRef,
    pub is_action: bool,
    pub is_commutative: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RefinedTypeInfo {
    pub base: TypeRef,
    pub predicates: Vec<crate::parser::ast::Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceInfo {
    pub super_traits: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    pub symbols: HashMap<String, Vec<SymbolInfo>>,
    pub enums: HashMap<String, Vec<String>>, // EnumName -> [Variant1, Variant2, ...]
    pub refined_types: HashMap<String, RefinedTypeInfo>,
    pub interfaces: HashMap<String, InterfaceInfo>,
    pub implementations: HashMap<String, Vec<String>>, // TypeName -> [Interface1, Interface2, ...]
    pub aliases: HashMap<String, String>, // AliasName -> TargetName
    pub exports: std::collections::HashSet<String>,
    pub imports: Vec<(String, Vec<String>)>, // (module_path, specific_items)
    pub parent: Option<Box<TypeEnv>>,
}

impl TypeEnv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn define_alias(&mut self, name: String, target: String) {
        self.aliases.insert(name, target);
    }

    pub fn define_interface(&mut self, name: String, super_traits: Vec<String>) {
        self.interfaces.insert(name, InterfaceInfo { super_traits });
    }

    pub fn define_implementation(&mut self, type_name: String, interface_name: String) {
        self.implementations.entry(type_name).or_default().push(interface_name);
    }

    pub fn implements(&self, type_name: &str, interface_name: &str) -> bool {
        if type_name == interface_name {
            return true;
        }

        // Check if type_name is an interface and extends interface_name
        if let Some(info) = self.interfaces.get(type_name).or_else(|| self.parent.as_ref().and_then(|p| p.interfaces.get(type_name))) {
            if info.super_traits.iter().any(|st| self.implements(st, interface_name)) {
                return true;
            }
        }

        // Check if type_name has an explicit implementation of interface_name
        if let Some(impls) = self.implementations.get(type_name).or_else(|| self.parent.as_ref().and_then(|p| p.implementations.get(type_name))) {
            if impls.iter().any(|iface| self.implements(iface, interface_name)) {
                return true;
            }
        }

        // Check parent scope
        if let Some(p) = &self.parent {
            if p.implements(type_name, interface_name) {
                return true;
            }
        }

        false
    }

    pub fn define_refined(&mut self, name: String, base: TypeRef, predicates: Vec<crate::parser::ast::Expr>) {
        self.refined_types.insert(name, RefinedTypeInfo { base, predicates });
    }

    pub fn lookup_refined(&self, name: &str) -> Option<&RefinedTypeInfo> {
        self.refined_types.get(name).or_else(|| {
            self.parent.as_ref().and_then(|p| p.lookup_refined(name))
        })
    }

    pub fn define_enum(&mut self, enum_name: String, variants: Vec<String>) {
        self.enums.insert(enum_name, variants);
    }

    pub fn define(&mut self, name: String, arity: usize, type_info: TypeRef, is_action: bool, is_commutative: bool) {
        let info = SymbolInfo {
            name: name.clone(),
            arity,
            type_info,
            is_action,
            is_commutative,
        };
        self.symbols.entry(name).or_default().push(info);
    }

    pub fn lookup_all(&self, name: &str) -> Option<&Vec<SymbolInfo>> {
        self.symbols.get(name).or_else(|| {
            self.parent.as_ref().and_then(|p| p.lookup_all(name))
        })
    }

    pub fn lookup_first(&self, name: &str) -> Option<&SymbolInfo> {
        self.lookup_all(name).and_then(|vec| vec.first())
    }
}
