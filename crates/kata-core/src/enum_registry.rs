//! Catálogo de variantes por enum.
//!
//! Populado no resolution (Pass 0) a partir de `EnumDecl`.
//! Consumido no inference para resolver patterns desqualificados
//! (`True` → `Variant { enum_name: "Boolean", variant: "True" }`)
//! e verificar exaustividade de match em `Sum`.

use std::collections::HashMap;

use crate::ty::Ty;

/// Informação de uma variante de enum.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantInfo {
    /// Nome da variante (ex: `Ok`, `Err`, `True`).
    pub name: String,
    /// Tipo do payload, se a variante carrega um valor.
    /// `None` = variante unitária (`True`, `False`, `None`).
    /// `Some(ty)` = variante com payload (`Ok(Int)`, `Some(Float)`).
    pub payload_ty: Option<Ty>,
    /// Nome da função predicado no DispatchTable.
    /// `None` = variante sem predicado (normal ou default/fallback).
    /// `Some(name)` = variante predicada (ex: `Magreza(< _ 18.5)`).
    pub predicate: Option<String>,
    /// Valor fixo constante. `None` = não é constante.
    /// `Some(text)` = variante constante (`OK(200)`), text é o literal bruto.
    /// O tipo do payload é inferido do literal e fica em `payload_ty`.
    /// O codegen usa o `payload_ty` para decodificar; o texto é o valor bruto.
    pub fixed_value: Option<String>,
}

/// Catálogo de variantes por enum.
#[derive(Debug, Clone, Default)]
pub struct EnumRegistry {
    /// enum_name → lista de variantes (em ordem de declaração).
    variants: HashMap<String, Vec<VariantInfo>>,
    /// enum_name → parâmetros de tipo (ex: `Result` → `["T", "E"]`).
    /// Vazio para enums não-genéricos.
    type_params: HashMap<String, Vec<String>>,
    /// enum_name → defaults dos type params (ex: `Result` → `[None, Some(Text)]`).
    /// Paralelo a `type_params`: `defaults[i]` é o default de `type_params[i]`.
    /// `None` = sem default (param obrigatório).
    defaults: HashMap<String, Vec<Option<Ty>>>,
}

impl EnumRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra um enum com suas variantes (payloads opcionais).
    pub fn register(&mut self, enum_name: &str, variants: Vec<VariantInfo>) {
        self.variants.insert(enum_name.to_string(), variants);
    }

    /// Registra um enum genérico com type params e variantes.
    /// `type_params` é a lista de nomes de parâmetros de tipo (ex: `["T", "E"]`).
    /// As variantes podem ter `payload_ty` com `Ty::Var("T")` etc.
    pub fn register_generic(
        &mut self,
        enum_name: &str,
        type_params: Vec<String>,
        variants: Vec<VariantInfo>,
    ) {
        self.type_params.insert(enum_name.to_string(), type_params);
        self.variants.insert(enum_name.to_string(), variants);
    }

    /// Registra um enum genérico com type params, defaults e variantes.
    /// `defaults` é paralelo a `type_params`: `defaults[i]` é o default de
    /// `type_params[i]`, ou `None` se o param é obrigatório.
    pub fn register_generic_with_defaults(
        &mut self,
        enum_name: &str,
        type_params: Vec<String>,
        defaults: Vec<Option<Ty>>,
        variants: Vec<VariantInfo>,
    ) {
        self.type_params.insert(enum_name.to_string(), type_params);
        self.defaults.insert(enum_name.to_string(), defaults);
        self.variants.insert(enum_name.to_string(), variants);
    }

    /// Retorna os defaults dos type params de um enum, se houver.
    pub fn defaults_of(&self, enum_name: &str) -> Option<&[Option<Ty>]> {
        self.defaults.get(enum_name).map(|v| v.as_slice())
    }

    /// Preenche type args faltantes com defaults.
    ///
    /// Se `type_args` tem menos elementos que `type_params`, os params
    /// faltantes são preenchidos com seus defaults (se houver).
    /// Se um param faltante não tem default, retorna `None` (erro: param
    /// obrigatório não fornecido).
    ///
    /// Ex: `Result` com `type_params=["T","E"]`, `defaults=[None, Some(Text)]`.
    /// `apply_defaults("Result", [Int])` → `Some([Int, Text])`.
    /// `apply_defaults("Result", [])` → `None` (T é obrigatório).
    pub fn apply_defaults(&self, enum_name: &str, type_args: &[Ty]) -> Option<Vec<Ty>> {
        let type_params = self.type_params_of(enum_name)?;
        let defaults = self.defaults_of(enum_name);

        if type_args.len() == type_params.len() {
            return Some(type_args.to_vec());
        }
        if type_args.len() > type_params.len() {
            return None; // args demais
        }

        let mut result = type_args.to_vec();
        for i in type_args.len()..type_params.len() {
            let default = defaults.and_then(|d| d.get(i).cloned().flatten());
            if let Some(default_ty) = default {
                result.push(default_ty);
            } else {
                return None; // param obrigatório sem valor
            }
        }
        Some(result)
    }

    /// Retorna os type params de um enum, se for genérico.
    pub fn type_params_of(&self, enum_name: &str) -> Option<&[String]> {
        self.type_params.get(enum_name).map(|v| v.as_slice())
    }

    /// Percorre um `Ty` recursivamente e aplica defaults em qualquer
    /// `Ty::Generic` que tenha menos args que type params.
    ///
    /// Ex: `Result::(Int)` → `Result::(Int, Text)` se o enum tem default
    /// `E=Text`. Recursa nos args do Generic, em Function, Tuple, List, etc.
    /// Se o enum não tem defaults ou já tem arity completo, retorna o tipo
    /// inalterado (apenas recursa nos filhos).
    pub fn expand_defaults(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Generic(name, args) => {
                // Recursa nos args primeiro.
                let expanded_args: Vec<Ty> =
                    args.iter().map(|a| self.expand_defaults(a)).collect();
                // Aplica defaults se o enum tem defaults registrados.
                match self.apply_defaults(name, &expanded_args) {
                    Some(full_args) => Ty::Generic(name.clone(), full_args),
                    None => Ty::Generic(name.clone(), expanded_args),
                }
            }
            Ty::Function(params, ret) => Ty::Function(
                params.iter().map(|p| self.expand_defaults(p)).collect(),
                Box::new(self.expand_defaults(ret)),
            ),
            Ty::Action(params, ret) => Ty::Action(
                params.iter().map(|p| self.expand_defaults(p)).collect(),
                Box::new(self.expand_defaults(ret)),
            ),
            Ty::Tuple(elements) => {
                Ty::Tuple(elements.iter().map(|e| self.expand_defaults(e)).collect())
            }
            Ty::List(inner) => Ty::List(Box::new(self.expand_defaults(inner))),
            Ty::Array(inner) => Ty::Array(Box::new(self.expand_defaults(inner))),
            Ty::Range(inner) => Ty::Range(Box::new(self.expand_defaults(inner))),
            Ty::Set(inner) => Ty::Set(Box::new(self.expand_defaults(inner))),
            Ty::Dict(k, v) => Ty::Dict(
                Box::new(self.expand_defaults(k)),
                Box::new(self.expand_defaults(v)),
            ),
            Ty::Sender(inner) => Ty::Sender(Box::new(self.expand_defaults(inner))),
            Ty::Receiver(inner) => Ty::Receiver(Box::new(self.expand_defaults(inner))),
            Ty::ReceiverFactory(inner) => {
                Ty::ReceiverFactory(Box::new(self.expand_defaults(inner)))
            }
            // Tipos atômicos — sem filhos para recursar.
            _ => ty.clone(),
        }
    }

    /// Verifica se um enum é genérico (tem type params).
    pub fn is_generic(&self, enum_name: &str) -> bool {
        self.type_params.contains_key(enum_name)
    }

    /// Substitui `Ty::Var(name)` por `type_args[i]` correspondente.
    /// Se `name` não está nos type_params, retorna o `Ty::Var` original
    /// (não deveria acontecer se o typeck está correto).
    fn substitute_vars(ty: &Ty, type_params: &[String], type_args: &[Ty]) -> Ty {
        match ty {
            Ty::Var(name) => {
                let idx = type_params.iter().position(|p| p == name);
                match idx {
                    Some(i) if i < type_args.len() => type_args[i].clone(),
                    _ => ty.clone(),
                }
            }
            Ty::Function(params, ret) => Ty::Function(
                params
                    .iter()
                    .map(|p| Self::substitute_vars(p, type_params, type_args))
                    .collect(),
                Box::new(Self::substitute_vars(ret, type_params, type_args)),
            ),
            Ty::Tuple(elements) => Ty::Tuple(
                elements
                    .iter()
                    .map(|e| Self::substitute_vars(e, type_params, type_args))
                    .collect(),
            ),
            Ty::Generic(name, args) => Ty::Generic(
                name.clone(),
                args.iter()
                    .map(|a| Self::substitute_vars(a, type_params, type_args))
                    .collect(),
            ),
            _ => ty.clone(),
        }
    }

    /// Instancia o payload de uma variante com type_args concretos.
    /// Ex: `instantiate_variant("Result", "Ok", [Int, Text])` → `Some(Ty::Prim(Int))`
    /// (substitui `Ty::Var("T")` por `Ty::Prim(Int)`).
    /// Retorna `None` se o enum/variante não existe ou não é genérico.
    pub fn instantiate_variant(
        &self,
        enum_name: &str,
        variant: &str,
        type_args: &[Ty],
    ) -> Option<Ty> {
        let type_params = self.type_params_of(enum_name)?;
        let payload_ty = self.payload_ty(enum_name, variant)?;
        Some(Self::substitute_vars(payload_ty, type_params, type_args))
    }

    /// Verifica se um nome é variante de um enum.
    pub fn is_variant(&self, enum_name: &str, variant: &str) -> bool {
        self.variants
            .get(enum_name)
            .is_some_and(|vs| vs.iter().any(|v| v.name == variant))
    }

    /// Lista os nomes das variantes de um enum (para verificação de exaustividade).
    pub fn variants_of(&self, enum_name: &str) -> Vec<&str> {
        self.variants
            .get(enum_name)
            .map(|vs| vs.iter().map(|v| v.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Busca o enum ao qual uma variante pertence.
    /// Retorna o nome do enum se `variant_name` for uma variante conhecida
    /// de algum enum. Usado para resolver patterns desqualificados.
    #[allow(dead_code)] // usado apenas em testes inline #[cfg(test)]
    pub(crate) fn find_enum_of_variant(&self, variant_name: &str) -> Option<&str> {
        self.variants
            .iter()
            .find(|(_, vs)| vs.iter().any(|v| v.name == variant_name))
            .map(|(enum_name, _)| enum_name.as_str())
    }

    /// Busca TODOS os enums que têm uma variante com o nome dado.
    /// Usado para resolver variantes desqualificadas em posição de expressão:
    /// se exatamente 1 enum tem a variante, resolve; se múltiplos, ambíguo.
    pub fn find_enums_with_variant(&self, variant_name: &str) -> Vec<&str> {
        self.variants
            .iter()
            .filter(|(_, vs)| vs.iter().any(|v| v.name == variant_name))
            .map(|(enum_name, _)| enum_name.as_str())
            .collect()
    }

    /// Retorna o índice de uma variante no enum (tag do Sum).
    /// `None` se o enum não existe ou a variante não pertence ao enum.
    pub fn variant_index(&self, enum_name: &str, variant: &str) -> Option<usize> {
        self.variants
            .get(enum_name)
            .and_then(|vs| vs.iter().position(|v| v.name == variant))
    }

    /// Retorna o tipo de payload de uma variante.
    /// `None` se a variante é unitária ou não existe.
    pub fn payload_ty(&self, enum_name: &str, variant: &str) -> Option<&Ty> {
        self.variants
            .get(enum_name)
            .and_then(|vs| vs.iter().find(|v| v.name == variant))
            .and_then(|v| v.payload_ty.as_ref())
    }

    /// Retorna o valor fixo constante de uma variante, se houver.
    /// `None` se a variante não é constante ou não existe.
    pub fn fixed_value(&self, enum_name: &str, variant: &str) -> Option<&str> {
        self.variants
            .get(enum_name)
            .and_then(|vs| vs.iter().find(|v| v.name == variant))
            .and_then(|v| v.fixed_value.as_deref())
    }

    /// Retorna informações completas de uma variante.
    #[allow(dead_code)]
    pub(crate) fn variant_info(&self, enum_name: &str, variant: &str) -> Option<&VariantInfo> {
        self.variants
            .get(enum_name)
            .and_then(|vs| vs.iter().find(|v| v.name == variant))
    }

    /// Lista os nomes de todos os enums registrados.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.variants.keys().map(|s| s.as_str())
    }

    /// Retorna todas as variantes de um enum com suas infos completas.
    pub fn all_variants(&self, enum_name: &str) -> Option<&[VariantInfo]> {
        self.variants.get(enum_name).map(|v| v.as_slice())
    }

    /// Mescla outro EnumRegistry neste. Enums do outro sobrescrevem
    /// enums com o mesmo nome. Usado para combinar prelude + user module.
    pub fn merge(&mut self, other: EnumRegistry) {
        for (name, variants) in other.variants {
            self.variants.insert(name, variants);
        }
        for (name, params) in other.type_params {
            self.type_params.insert(name, params);
        }
        for (name, defaults) in other.defaults {
            self.defaults.insert(name, defaults);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_query() {
        let mut registry = EnumRegistry::new();
        registry.register(
            "Boolean",
            vec![
                VariantInfo {
                    name: "True".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
                VariantInfo {
                    name: "False".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
            ],
        );

        assert!(registry.is_variant("Boolean", "True"));
        assert!(registry.is_variant("Boolean", "False"));
        assert!(!registry.is_variant("Boolean", "Maybe"));

        let variants = registry.variants_of("Boolean");
        assert_eq!(variants, &["True", "False"]);
    }

    #[test]
    fn find_enum_of_variant() {
        let mut registry = EnumRegistry::new();
        registry.register(
            "Boolean",
            vec![
                VariantInfo {
                    name: "True".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
                VariantInfo {
                    name: "False".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
            ],
        );

        assert_eq!(registry.find_enum_of_variant("True"), Some("Boolean"));
        assert_eq!(registry.find_enum_of_variant("False"), Some("Boolean"));
        assert_eq!(registry.find_enum_of_variant("Maybe"), None);
    }

    #[test]
    fn find_enums_with_variant() {
        let mut registry = EnumRegistry::new();
        registry.register(
            "Boolean",
            vec![
                VariantInfo {
                    name: "True".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
                VariantInfo {
                    name: "False".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
            ],
        );
        registry.register(
            "Flag",
            vec![
                VariantInfo {
                    name: "True".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
                VariantInfo {
                    name: "Off".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
            ],
        );

        // "True" existe em Boolean e Flag → ambíguo
        let mut enums = registry.find_enums_with_variant("True");
        enums.sort();
        assert_eq!(enums, vec!["Boolean", "Flag"]);

        // "False" só em Boolean
        assert_eq!(registry.find_enums_with_variant("False"), vec!["Boolean"]);

        // "Off" só em Flag
        assert_eq!(registry.find_enums_with_variant("Off"), vec!["Flag"]);

        // "Maybe" em nenhum
        assert!(registry.find_enums_with_variant("Maybe").is_empty());
    }

    #[test]
    fn unknown_enum_returns_empty() {
        let registry = EnumRegistry::new();
        assert!(registry.variants_of("NonExistent").is_empty());
        assert!(!registry.is_variant("NonExistent", "Anything"));
    }

    #[test]
    fn variant_index_and_payload() {
        let mut registry = EnumRegistry::new();
        registry.register(
            "Result",
            vec![
                VariantInfo {
                    name: "Ok".into(),
                    payload_ty: Some(Ty::int()),
                    predicate: None,
                    fixed_value: None,
                },
                VariantInfo {
                    name: "Err".into(),
                    payload_ty: Some(Ty::text()),
                    predicate: None,
                    fixed_value: None,
                },
            ],
        );

        assert_eq!(registry.variant_index("Result", "Ok"), Some(0));
        assert_eq!(registry.variant_index("Result", "Err"), Some(1));
        assert_eq!(registry.variant_index("Result", "Maybe"), None);

        assert_eq!(registry.payload_ty("Result", "Ok"), Some(&Ty::int()));
        assert_eq!(registry.payload_ty("Result", "Err"), Some(&Ty::text()));
        assert_eq!(registry.payload_ty("Result", "Maybe"), None);
    }

    #[test]
    fn unit_variant_has_no_payload() {
        let mut registry = EnumRegistry::new();
        registry.register(
            "Optional",
            vec![
                VariantInfo {
                    name: "Some".into(),
                    payload_ty: Some(Ty::int()),
                    predicate: None,
                    fixed_value: None,
                },
                VariantInfo {
                    name: "None".into(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
            ],
        );

        assert_eq!(registry.payload_ty("Optional", "Some"), Some(&Ty::int()));
        assert_eq!(registry.payload_ty("Optional", "None"), None);
    }
}
