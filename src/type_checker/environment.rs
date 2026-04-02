use std::collections::HashMap;
use crate::parser::ast::TypeRef;

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInfo {
    pub name: String,
    pub arity: usize,
    pub type_info: TypeRef,
    pub is_action: bool,
    pub is_commutative: bool,
    pub ffi_name: Option<String>,
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
    pub interface_methods: HashMap<String, Vec<String>>, // InterfaceName -> [Method1, Method2]
    pub type_methods: HashMap<String, Vec<String>>,      // TypeName -> [Method1, Method2]
    pub exports: std::collections::HashSet<String>,
    pub imports: Vec<(String, Vec<(String, Option<String>)>)>, // (module_path, [(item, alias)])
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

    pub fn define(&mut self, name: String, arity: usize, type_info: TypeRef, is_action: bool, is_commutative: bool, ffi_name: Option<String>) {
        let info = SymbolInfo {
            name: name.clone(),
            arity,
            type_info,
            is_action,
            is_commutative,
            ffi_name,
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

        pub fn expand_exports(&mut self) {
        let mut new_exports = Vec::new();
        
        for exp in &self.exports {
            if let Some(methods) = self.interface_methods.get(exp) {
                new_exports.extend(methods.clone());
                
            }
            if let Some(methods) = self.type_methods.get(exp) {
                new_exports.extend(methods.clone());
                
            }
        }
        for exp in new_exports {
            self.exports.insert(exp);
        }
    }

    pub fn import_from(&mut self, other: &TypeEnv, module_name: &str, specific: &[(String, Option<String>)]) {
        let is_namespace = specific.is_empty();
        let prefix = if is_namespace { format!("{}.", module_name) } else { "".to_string() };

        let mut expanded_specific = specific.to_vec();
        if !is_namespace {
            let mut new_specific = Vec::new();
            for (item, _) in specific {
                if let Some(methods) = other.interface_methods.get(item) {
                    for m in methods { new_specific.push((m.clone(), None)); }
                }
                if let Some(methods) = other.type_methods.get(item) {
                    for m in methods { new_specific.push((m.clone(), None)); }
                }
            }
            expanded_specific.extend(new_specific);
        }
        let specific_slice = &expanded_specific;

        // Helper to check if an item is exported from `other`
        let is_exported = |name: &str| -> bool {
            other.exports.contains(name)
        };

        // Helper to get the imported name
        let get_imported_name = |name: &str| -> Option<String> {
            if !is_exported(name) { return None; }
            if is_namespace { return Some(format!("{}{}", prefix, name)); }
            
            for (item, alias) in specific_slice {
                if item == name {
                    return Some(alias.clone().unwrap_or(name.to_string()));
                }
            }
            None
        };

        // Copy Symbols
        for (name, infos) in &other.symbols {
            if let Some(imported_name) = get_imported_name(name) {
                self.symbols.insert(imported_name, infos.clone());
            }
        }

        // Copy Enums
        for (name, variants) in &other.enums {
            if let Some(imported_name) = get_imported_name(name) {
                self.enums.insert(imported_name, variants.clone());
                for var in variants {
                    if let Some(infos) = other.symbols.get(var) {
                        let imported_var = if is_namespace {
                            format!("{}{}", prefix, var)
                        } else {
                            var.to_string()
                        };
                        self.symbols.insert(imported_var, infos.clone());
                    }
                }
            }
        }

        // Copy Refined Types
        for (name, info) in &other.refined_types {
            if let Some(imported_name) = get_imported_name(name) {
                self.refined_types.insert(imported_name, info.clone());
            }
        }

        // Copy Interface Methods
        for (iface, methods) in &other.interface_methods {
            if let Some(imported_iface) = get_imported_name(iface) {
                let imported_methods: Vec<String> = methods.iter().filter_map(|m| get_imported_name(m)).collect();
                self.interface_methods.insert(imported_iface, imported_methods);
            }
        }

        // Copy Type Methods
        for (ty, methods) in &other.type_methods {
            if let Some(imported_ty) = get_imported_name(ty) {
                let imported_methods: Vec<String> = methods.iter().filter_map(|m| get_imported_name(m)).collect();
                self.type_methods.insert(imported_ty, imported_methods);
            }
        }

        // Copy Interfaces
        for (name, info) in &other.interfaces {
            if let Some(imported_name) = get_imported_name(name) {
                self.interfaces.insert(imported_name, info.clone());
            }
        }

        // Copy Aliases
        for (name, target) in &other.aliases {
            if let Some(imported_name) = get_imported_name(name) {
                self.aliases.insert(imported_name, target.clone());
            }
        }

        // Implementations (Global Contracts)
        for (type_name, ifaces) in &other.implementations {
            let imported_type_name = if let Some(n) = get_imported_name(type_name) { n } else { continue; };
            
            for iface in ifaces {
                let imported_iface_name = if is_namespace {
                    if is_exported(iface) { format!("{}.", module_name) + iface } else { iface.clone() }
                } else {
                    iface.clone()
                };
                self.define_implementation(imported_type_name.clone(), imported_iface_name);
            }
        }
    }

}
