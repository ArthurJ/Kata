//! Catálogo de variantes por enum, com rastreio de origem (origin).
//!
//! Populado no resolution (Pass 0) a partir de `EnumDecl`.
//! Consumido no inference para resolver patterns desqualificados
//! (`True` → `Variant { enum_name: "Boolean", variant: "True" }`)
//! e verificar exaustividade de match em `Sum`.
//!
//! Cada enum é registrado com `origin` (módulo de origem: "core", "my_module", etc).
//! Lookups não-qualificados resolvem a origin automaticamente quando há apenas uma;
//! quando há múltiplas origins (nome ambíguo), `is_ambiguous` retorna true e
//! o caller deve usar `*_with_origin` para desambiguar.

use std::collections::{HashMap, HashSet};

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

/// Catálogo de variantes por enum, com rastreio de origem.
#[derive(Debug, Clone, Default)]
pub struct EnumRegistry {
    /// (origin, enum_name) → lista de variantes (em ordem de declaração).
    variants: HashMap<(String, String), Vec<VariantInfo>>,
    /// (origin, enum_name) → parâmetros de tipo (ex: `Result` → `["T", "E"]`).
    /// Vazio para enums não-genéricos.
    type_params: HashMap<(String, String), Vec<String>>,
    /// (origin, enum_name) → defaults dos type params.
    /// Paralelo a `type_params`: `defaults[i]` é o default de `type_params[i]`.
    /// `None` = sem default (param obrigatório).
    defaults: HashMap<(String, String), Vec<Option<Ty>>>,
    /// enum_name → conjunto de origins que definem este enum.
    origins: HashMap<String, HashSet<String>>,
    /// Nomes ambíguos (definidos em múltiplas origins).
    ambiguous: HashSet<String>,
}

impl EnumRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Registro ──────────────────────────────────────────

    /// Registra um enum com suas variantes (payloads opcionais).
    pub fn register(&mut self, origin: &str, enum_name: &str, variants: Vec<VariantInfo>) {
        let key = (origin.to_string(), enum_name.to_string());
        self.variants.insert(key, variants);
        self.track_origin(enum_name, origin);
    }

    /// Registra um enum genérico com type params e variantes.
    pub fn register_generic(
        &mut self,
        origin: &str,
        enum_name: &str,
        type_params: Vec<String>,
        variants: Vec<VariantInfo>,
    ) {
        let key = (origin.to_string(), enum_name.to_string());
        self.type_params.insert(key.clone(), type_params);
        self.variants.insert(key, variants);
        self.track_origin(enum_name, origin);
    }

    /// Registra um enum genérico com type params, defaults e variantes.
    pub fn register_generic_with_defaults(
        &mut self,
        origin: &str,
        enum_name: &str,
        type_params: Vec<String>,
        defaults: Vec<Option<Ty>>,
        variants: Vec<VariantInfo>,
    ) {
        let key = (origin.to_string(), enum_name.to_string());
        self.type_params.insert(key.clone(), type_params);
        self.defaults.insert(key.clone(), defaults);
        self.variants.insert(key, variants);
        self.track_origin(enum_name, origin);
    }

    /// Rastreia a origin de um enum e marca ambíguo se >1 origin.
    fn track_origin(&mut self, enum_name: &str, origin: &str) {
        let origins = self.origins.entry(enum_name.to_string()).or_default();
        origins.insert(origin.to_string());
        if origins.len() > 1 {
            self.ambiguous.insert(enum_name.to_string());
        }
    }

    // ── Resolução de origin ───────────────────────────────

    /// Retorna true se o enum_name é ambíguo (definido em múltiplas origins).
    pub fn is_ambiguous(&self, enum_name: &str) -> bool {
        self.ambiguous.contains(enum_name)
    }

    /// Retorna as origins que definem este enum.
    pub fn origins_of(&self, enum_name: &str) -> Vec<&str> {
        self.origins
            .get(enum_name)
            .map(|s| s.iter().map(|o| o.as_str()).collect())
            .unwrap_or_default()
    }

    /// Resolve a origin de um enum não-qualificado.
    ///
    /// Retorna `Some(origin)` se há exatamente uma origin, ou se há
    /// múltiplas mas uma delas é `"__local__"` (preferindo o escopo do
    /// usuário sobre o prelude). Retorna `None` se ambíguo (múltiplas
    /// origins, nenhuma é `"__local__"`) ou não existe.
    ///
    /// Isto implementa shadowing: o usuário pode redefinir `Result` e
    /// usar `Result::Ok 42` sem qualificar (resolve o do usuário), enquanto
    /// o prelude usa `core.Result::Err` qualificado (resolve o do prelude).
    pub fn resolve_origin(&self, enum_name: &str) -> Option<&str> {
        self.origins.get(enum_name).and_then(|origins| {
            if origins.len() == 1 {
                origins.iter().next().map(|s| s.as_str())
            } else if origins.contains("__local__") {
                // Shadowing: usuário define localmente, prelude coexiste.
                // Preferir o do usuário para lookups não-qualificados.
                Some("__local__")
            } else {
                None
            }
        })
    }

    // ── Defaults ──────────────────────────────────────────

    /// Retorna os defaults dos type params de um enum, se houver.
    /// Usa `resolve_origin` para encontrar a origin.
    pub fn defaults_of(&self, enum_name: &str) -> Option<&[Option<Ty>]> {
        let origin = self.resolve_origin(enum_name)?;
        let key = (origin.to_string(), enum_name.to_string());
        self.defaults.get(&key).map(|v| v.as_slice())
    }

    /// Retorna os defaults com origin explícita.
    pub fn defaults_of_with_origin(&self, origin: &str, enum_name: &str) -> Option<&[Option<Ty>]> {
        let key = (origin.to_string(), enum_name.to_string());
        self.defaults.get(&key).map(|v| v.as_slice())
    }

    /// Preenche type args faltantes com defaults.
    pub fn apply_defaults(&self, enum_name: &str, type_args: &[Ty]) -> Option<Vec<Ty>> {
        let type_params = self.type_params_of(enum_name)?;
        let defaults = self.defaults_of(enum_name);
        Self::do_apply_defaults(type_params, defaults, type_args)
    }

    /// `apply_defaults` com origin explícita.
    pub fn apply_defaults_with_origin(
        &self,
        origin: &str,
        enum_name: &str,
        type_args: &[Ty],
    ) -> Option<Vec<Ty>> {
        let type_params = self.type_params_of_with_origin(origin, enum_name)?;
        let defaults = self.defaults_of_with_origin(origin, enum_name);
        Self::do_apply_defaults(type_params, defaults, type_args)
    }

    fn do_apply_defaults(
        type_params: &[String],
        defaults: Option<&[Option<Ty>]>,
        type_args: &[Ty],
    ) -> Option<Vec<Ty>> {
        if type_args.len() == type_params.len() {
            return Some(type_args.to_vec());
        }
        if type_args.len() > type_params.len() {
            return None;
        }
        let mut result = type_args.to_vec();
        for i in type_args.len()..type_params.len() {
            let default = defaults.and_then(|d| d.get(i).and_then(|opt| opt.clone()));
            if let Some(default_ty) = default {
                result.push(default_ty);
            } else {
                return None;
            }
        }
        Some(result)
    }

    // ── Type params ───────────────────────────────────────

    /// Retorna os type params de um enum, se for genérico.
    pub fn type_params_of(&self, enum_name: &str) -> Option<&[String]> {
        let origin = self.resolve_origin(enum_name)?;
        let key = (origin.to_string(), enum_name.to_string());
        self.type_params.get(&key).map(|v| v.as_slice())
    }

    /// `type_params_of` com origin explícita.
    pub fn type_params_of_with_origin(&self, origin: &str, enum_name: &str) -> Option<&[String]> {
        let key = (origin.to_string(), enum_name.to_string());
        self.type_params.get(&key).map(|v| v.as_slice())
    }

    /// Percorre um `Ty` recursivamente e aplica defaults em qualquer
    /// `Ty::Generic` que tenha menos args que type params.
    pub fn expand_defaults(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Generic(name, args) => {
                let expanded_args: Vec<Ty> = args.iter().map(|a| self.expand_defaults(a)).collect();
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
            _ => ty.clone(),
        }
    }

    // ── Genérico ──────────────────────────────────────────

    /// Verifica se um enum é genérico (tem type params).
    pub fn is_generic(&self, enum_name: &str) -> bool {
        self.type_params_of(enum_name).is_some()
    }

    /// `is_generic` com origin explícita.
    pub fn is_generic_with_origin(&self, origin: &str, enum_name: &str) -> bool {
        self.type_params_of_with_origin(origin, enum_name).is_some()
    }

    /// Substitui `Ty::Var(name)` por `type_args[i]` correspondente.
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

    // ── Consulta de variantes ─────────────────────────────

    /// Verifica se um nome é variante de um enum.
    pub fn is_variant(&self, enum_name: &str, variant: &str) -> bool {
        let origin = match self.resolve_origin(enum_name) {
            Some(o) => o,
            None => return false,
        };
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .is_some_and(|vs| vs.iter().any(|v| v.name == variant))
    }

    /// `is_variant` com origin explícita.
    pub fn is_variant_with_origin(&self, origin: &str, enum_name: &str, variant: &str) -> bool {
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .is_some_and(|vs| vs.iter().any(|v| v.name == variant))
    }

    /// Lista os nomes das variantes de um enum.
    pub fn variants_of(&self, enum_name: &str) -> Vec<&str> {
        let origin = match self.resolve_origin(enum_name) {
            Some(o) => o,
            None => return Vec::new(),
        };
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .map(|vs| vs.iter().map(|v| v.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Busca o enum ao qual uma variante pertence (não-qualificado).
    #[allow(dead_code)]
    pub(crate) fn find_enum_of_variant(&self, variant_name: &str) -> Option<&str> {
        self.variants
            .iter()
            .filter(|(_, vs)| vs.iter().any(|v| v.name == variant_name))
            .map(|((_, name), _)| name.as_str())
            .next()
    }

    /// Busca TODOS os enums que têm uma variante com o nome dado.
    pub fn find_enums_with_variant(&self, variant_name: &str) -> Vec<&str> {
        let mut result: Vec<&str> = self
            .variants
            .iter()
            .filter(|(_, vs)| vs.iter().any(|v| v.name == variant_name))
            .map(|((_, name), _)| name.as_str())
            .collect();
        result.sort();
        result.dedup();
        result
    }

    /// Retorna o índice de uma variante no enum (tag do Sum).
    pub fn variant_index(&self, enum_name: &str, variant: &str) -> Option<usize> {
        let origin = self.resolve_origin(enum_name)?;
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .and_then(|vs| vs.iter().position(|v| v.name == variant))
    }

    /// `variant_index` com origin explícita.
    pub fn variant_index_with_origin(
        &self,
        origin: &str,
        enum_name: &str,
        variant: &str,
    ) -> Option<usize> {
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .and_then(|vs| vs.iter().position(|v| v.name == variant))
    }

    /// Retorna o tipo de payload de uma variante.
    pub fn payload_ty(&self, enum_name: &str, variant: &str) -> Option<&Ty> {
        let origin = self.resolve_origin(enum_name)?;
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .and_then(|vs| vs.iter().find(|v| v.name == variant))
            .and_then(|v| v.payload_ty.as_ref())
    }

    /// `payload_ty` com origin explícita.
    pub fn payload_ty_with_origin(
        &self,
        origin: &str,
        enum_name: &str,
        variant: &str,
    ) -> Option<&Ty> {
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .and_then(|vs| vs.iter().find(|v| v.name == variant))
            .and_then(|v| v.payload_ty.as_ref())
    }

    /// Retorna o valor fixo constante de uma variante.
    pub fn fixed_value(&self, enum_name: &str, variant: &str) -> Option<&str> {
        let origin = self.resolve_origin(enum_name)?;
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .and_then(|vs| vs.iter().find(|v| v.name == variant))
            .and_then(|v| v.fixed_value.as_deref())
    }

    /// `fixed_value` com origin explícita.
    pub fn fixed_value_with_origin(
        &self,
        origin: &str,
        enum_name: &str,
        variant: &str,
    ) -> Option<&str> {
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .and_then(|vs| vs.iter().find(|v| v.name == variant))
            .and_then(|v| v.fixed_value.as_deref())
    }

    /// Retorna informações completas de uma variante.
    #[allow(dead_code)]
    pub(crate) fn variant_info(&self, enum_name: &str, variant: &str) -> Option<&VariantInfo> {
        let origin = self.resolve_origin(enum_name)?;
        let key = (origin.to_string(), enum_name.to_string());
        self.variants
            .get(&key)
            .and_then(|vs| vs.iter().find(|v| v.name == variant))
    }

    /// Lista os nomes de todos os enums registrados (não-ambíguos).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.origins.keys().map(|s| s.as_str())
    }

    /// Retorna todas as variantes de um enum.
    pub fn all_variants(&self, enum_name: &str) -> Option<&[VariantInfo]> {
        let origin = self.resolve_origin(enum_name)?;
        let key = (origin.to_string(), enum_name.to_string());
        self.variants.get(&key).map(|v| v.as_slice())
    }

    /// `all_variants` com origin explícita.
    pub fn all_variants_with_origin(
        &self,
        origin: &str,
        enum_name: &str,
    ) -> Option<&[VariantInfo]> {
        let key = (origin.to_string(), enum_name.to_string());
        self.variants.get(&key).map(|v| v.as_slice())
    }

    // ── Merge ─────────────────────────────────────────────

    /// Mescla outro EnumRegistry neste.
    /// Enums de origins diferentes coexistem; nomes com múltiplas origins
    /// são marcados como ambíguos. Enums da mesma origin são sobrescritos
    /// (re-registro no mesmo módulo).
    pub fn merge(&mut self, other: EnumRegistry) {
        for ((origin, name), variants) in other.variants {
            let key = (origin.clone(), name.clone());
            self.variants.insert(key, variants);
            self.track_origin(&name, &origin);
        }
        for ((origin, name), params) in other.type_params {
            let key = (origin, name);
            self.type_params.insert(key, params);
        }
        for ((origin, name), defaults) in other.defaults {
            let key = (origin, name);
            self.defaults.insert(key, defaults);
        }
    }

    /// Filtra enums mantendo apenas aqueles cujo nome está no `closure`
    /// ou cuja origin é `core` (prelude). Usado por `filter_exports`.
    pub fn retain_by_closure(&mut self, closure: &std::collections::HashSet<String>) {
        self.variants.retain(|(_, name), _| {
            closure.contains(name) || {
                // Manter se qualquer origin for core
                self.origins
                    .get(name)
                    .is_some_and(|origins| origins.contains("core"))
            }
        });
        self.type_params.retain(|(_, name), _| {
            closure.contains(name) || {
                self.origins
                    .get(name)
                    .is_some_and(|origins| origins.contains("core"))
            }
        });
        self.defaults.retain(|(_, name), _| {
            closure.contains(name) || {
                self.origins
                    .get(name)
                    .is_some_and(|origins| origins.contains("core"))
            }
        });
        // Reconstruir origins e ambiguous com base nos sobreviventes
        self.origins.clear();
        self.ambiguous.clear();
        for (origin, name) in self.variants.keys() {
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

    fn v(name: &str) -> VariantInfo {
        VariantInfo {
            name: name.into(),
            payload_ty: None,
            predicate: None,
            fixed_value: None,
        }
    }

    fn v_with_payload(name: &str, ty: Ty) -> VariantInfo {
        VariantInfo {
            name: name.into(),
            payload_ty: Some(ty),
            predicate: None,
            fixed_value: None,
        }
    }

    #[test]
    fn register_and_query() {
        let mut registry = EnumRegistry::new();
        registry.register("core", "Boolean", vec![v("True"), v("False")]);

        assert!(registry.is_variant("Boolean", "True"));
        assert!(registry.is_variant("Boolean", "False"));
        assert!(!registry.is_variant("Boolean", "Maybe"));

        let variants = registry.variants_of("Boolean");
        assert_eq!(variants, &["True", "False"]);
    }

    #[test]
    fn find_enum_of_variant() {
        let mut registry = EnumRegistry::new();
        registry.register("core", "Boolean", vec![v("True"), v("False")]);

        assert_eq!(registry.find_enum_of_variant("True"), Some("Boolean"));
        assert_eq!(registry.find_enum_of_variant("False"), Some("Boolean"));
        assert_eq!(registry.find_enum_of_variant("Maybe"), None);
    }

    #[test]
    fn find_enums_with_variant() {
        let mut registry = EnumRegistry::new();
        registry.register("core", "Boolean", vec![v("True"), v("False")]);
        registry.register("user", "Flag", vec![v("True"), v("Off")]);

        let mut enums = registry.find_enums_with_variant("True");
        enums.sort();
        assert_eq!(enums, vec!["Boolean", "Flag"]);

        assert_eq!(registry.find_enums_with_variant("False"), vec!["Boolean"]);
        assert_eq!(registry.find_enums_with_variant("Off"), vec!["Flag"]);
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
            "core",
            "Result",
            vec![
                v_with_payload("Ok", Ty::int()),
                v_with_payload("Err", Ty::text()),
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
            "core",
            "Optional",
            vec![v_with_payload("Some", Ty::int()), v("None")],
        );

        assert_eq!(registry.payload_ty("Optional", "Some"), Some(&Ty::int()));
        assert_eq!(registry.payload_ty("Optional", "None"), None);
    }

    // ── Testes de origin + ambiguous ──────────────────────

    #[test]
    fn merge_different_origins_marks_ambiguous() {
        let mut prelude = EnumRegistry::new();
        prelude.register_generic_with_defaults(
            "core",
            "Result",
            vec!["T".into(), "E".into()],
            vec![None, Some(Ty::text())],
            vec![
                v_with_payload("Ok", Ty::Var("T".into())),
                v_with_payload("Err", Ty::Var("E".into())),
            ],
        );

        let mut user = EnumRegistry::new();
        user.register(
            "user",
            "Result",
            vec![
                v_with_payload("Ok", Ty::int()),
                v_with_payload("Err", Ty::int()),
            ],
        );

        prelude.merge(user);

        // Result é ambíguo (definido em core + user)
        assert!(prelude.is_ambiguous("Result"));
        assert_eq!(prelude.origins_of("Result").len(), 2);

        // Unqualified lookup falha (ambíguo)
        assert!(!prelude.is_variant("Result", "Ok"));
        assert_eq!(prelude.payload_ty("Result", "Ok"), None);

        // Qualified lookup funciona
        assert!(prelude.is_variant_with_origin("core", "Result", "Ok"));
        assert!(prelude.is_variant_with_origin("user", "Result", "Ok"));
        assert_eq!(
            prelude.payload_ty_with_origin("core", "Result", "Err"),
            Some(&Ty::Var("E".into()))
        );
        assert_eq!(
            prelude.payload_ty_with_origin("user", "Result", "Err"),
            Some(&Ty::int())
        );
    }

    #[test]
    fn merge_same_origin_overwrites() {
        let mut registry = EnumRegistry::new();
        registry.register("core", "Result", vec![v("Ok"), v("Err")]);

        let mut update = EnumRegistry::new();
        update.register("core", "Result", vec![v("Success"), v("Failure")]);

        registry.merge(update);

        // Same origin — overwritten, not ambiguous
        assert!(!registry.is_ambiguous("Result"));
        assert!(registry.is_variant("Result", "Success"));
        assert!(!registry.is_variant("Result", "Ok"));
    }

    #[test]
    fn resolve_origin_single() {
        let mut registry = EnumRegistry::new();
        registry.register("core", "Boolean", vec![v("True")]);

        assert_eq!(registry.resolve_origin("Boolean"), Some("core"));
        assert_eq!(registry.resolve_origin("NonExistent"), None);
    }

    #[test]
    fn resolve_origin_ambiguous() {
        let mut registry = EnumRegistry::new();
        registry.register("core", "Result", vec![v("Ok")]);
        registry.register("user", "Result", vec![v("Err")]);

        // Ambiguous — resolve_origin returns None
        assert_eq!(registry.resolve_origin("Result"), None);
        assert!(registry.is_ambiguous("Result"));
    }
}
