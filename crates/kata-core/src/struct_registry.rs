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

use std::collections::{BTreeMap, BTreeSet};

use crate::ty::Ty;

/// Chave interna do `StructRegistry` para distinguir tipos comuns de
/// instâncias de família de refined polimórfico e de structs paramétricos.
///
/// - `Plain("Pessoa")` — struct comum ou refined concreto.
/// - `Family("NonZero")` — referência a família polimórfica
///   (`data (NUM, ...) as NonZero`). Expandir em instâncias concretas
///   antes do dispatch.
/// - `Instance("NonZero", "Int")` — instância de `data (NUM, ...) as NonZero`
///   para o tipo concreto `Int`. O nome público é `"NonZero"`.
/// - `Generic("Complex", [Ty::Prim(Int), Ty::Prim(Int)])` — struct
///   paramétrico instanciado com type args concretos. O nome público é
///   `"Complex"`. Preserva o invariante `Ty::Struct` ↔ data.
///
/// `Ty::Struct` continua carregando o nome público (`"NonZero"`). A
/// distinção família vs concreto vs genérico é confinada ao `StructRegistry`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StructKey {
    /// Tipo comum: "Pessoa", "Float", "NonZero" (refined concreto).
    Plain(String),
    /// Família polimórfica: "NonZero" = `data (NUM, ...) as NonZero`.
    /// Não há struct concreto com este nome — é uma referência à família
    /// que deve ser expandida em `Instance` concretas.
    Family(String),
    /// Instância de família: ("NonZero", "Int") = NonZero sobre Int.
    Instance(String, String),
    /// Struct paramétrico instanciado: ("Complex", [Int, Int]).
    /// O layout é idêntico para quaisquer type args (offset = i * 8),
    /// mas os tipos anotados dos fields variam para o type checker.
    Generic(String, Vec<Ty>),
}

impl StructKey {
    /// Nome público (ex: "NonZero" para Plain, Family e Instance).
    pub fn name(&self) -> &str {
        match self {
            StructKey::Plain(n) => n,
            StructKey::Family(n) => n,
            StructKey::Instance(n, _) => n,
            StructKey::Generic(n, _) => n,
        }
    }

    /// Tipo concreto da instância, se aplicável.
    pub fn concrete_type(&self) -> Option<&str> {
        match self {
            StructKey::Plain(_) | StructKey::Family(_) | StructKey::Generic(..) => None,
            StructKey::Instance(_, t) => Some(t),
        }
    }
}

impl PartialOrd for StructKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StructKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Ordenação consistente com Eq: comparar por (discriminante, nome,
        // dados relevantes). Generic(args) compara por nome; args não
        // participam (Ty não impl Ord) — mas args participam de Eq/Hash,
        // então Generic com mesmo nome mas args diferentes são entradas
        // distintas no BTreeMap (Eq as distingue mesmo que Ord as considere
        // iguais por nome).
        match (self, other) {
            (StructKey::Plain(a), StructKey::Plain(b)) => a.cmp(b),
            (StructKey::Family(a), StructKey::Family(b)) => a.cmp(b),
            (StructKey::Instance(a, c1), StructKey::Instance(b, c2)) => {
                a.cmp(b).then_with(|| c1.cmp(c2))
            }
            (StructKey::Generic(a, _), StructKey::Generic(b, _)) => a.cmp(b),
            // Discriminante ordem: Plain < Family < Instance < Generic
            (StructKey::Plain(_), _) => std::cmp::Ordering::Less,
            (_, StructKey::Plain(_)) => std::cmp::Ordering::Greater,
            (StructKey::Family(_), _) => std::cmp::Ordering::Less,
            (_, StructKey::Family(_)) => std::cmp::Ordering::Greater,
            (StructKey::Instance(_, _), _) => std::cmp::Ordering::Less,
            (_, StructKey::Instance(_, _)) => std::cmp::Ordering::Greater,
        }
    }
}

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

/// Declaração de type param de um struct paramétrico.
/// `data Complex (re::T im::T) where T implements SCALAR` produz
/// `TypeParamDecl { name: "T", bound: Some("SCALAR") }`.
/// `data Par (first::A second::B)` produz dois params com `bound: None`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParamDecl {
    /// Nome do type param (PascalCase: T, A, B, etc.).
    pub name: String,
    /// Interface do bound (`Some("SCALAR")` = T implementa SCALAR).
    /// `None` = type param livre (sem bound).
    pub bound: Option<String>,
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
    /// Se este StructInfo é uma instância de uma família de refined polimórfico.
    /// `Some("NonZero")` = instância de `data (NUM, ...) as NonZero`.
    /// `None` = struct normal ou refined concreto (não-polimórfico).
    pub is_instance_of: Option<String>,
    /// Type params do struct paramétrico.
    /// `None` = struct monomórfico (não-genérico).
    /// `Some(vec)` = struct paramétrico com type params detectados
    /// por PascalCase em posição de tipo nos fields.
    pub type_params: Option<Vec<TypeParamDecl>>,
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

/// Catálogo de structs por nome, com rastreio de origem (origin).
#[derive(Debug, Clone, Default)]
pub struct StructRegistry {
    /// (origin, StructKey) → StructInfo.
    /// `StructKey::Plain(name)` para structs comuns e refined concretos.
    /// `StructKey::Instance(family, concrete)` para instâncias de família.
    structs: BTreeMap<(String, StructKey), StructInfo>,
    /// struct_name → conjunto de origins que definem este struct.
    origins: BTreeMap<String, BTreeSet<String>>,
    /// Nomes ambíguos (definidos em múltiplas origins).
    ambiguous: BTreeSet<String>,
    /// family_name → nome da interface sobre a qual a família é definida.
    /// Ex: "NonZero" → "NUM". Populado em pass0 quando `data (IFACE, ...) as Fam`
    /// é processado. Usado por `families_over_iface` para encontrar famílias
    /// que precisam ser estendidas quando um novo implementor de IFACE aparece.
    family_iface: BTreeMap<String, String>,
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
        let key = (origin.to_string(), StructKey::Plain(name.to_string()));
        self.structs.insert(
            key,
            StructInfo {
                name: name.to_string(),
                fields,
                alias_of,
                predicates: None,
                is_instance_of: None,
                type_params: None,
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
        let key = (origin.to_string(), StructKey::Plain(name.to_string()));
        self.structs.insert(
            key,
            StructInfo {
                name: name.to_string(),
                fields: Vec::new(),
                alias_of: Some(alias_of.to_string()),
                predicates: Some(predicates),
                is_instance_of: None,
                type_params: None,
            },
        );
        self.track_origin(name, origin);
    }

    /// Registra uma instância de família de refined polimórfico.
    ///
    /// `data (NUM, != _ (zero _)) as NonZero` com `concrete_type = "Int"`
    /// registra `StructKey::Instance("NonZero", "Int")` com
    /// `alias_of = "Int"`, `is_instance_of = Some("NonZero")`.
    pub fn register_refined_instance(
        &mut self,
        origin: &str,
        family_name: &str,
        concrete_type: &str,
        predicates: Vec<String>,
    ) {
        let key = (
            origin.to_string(),
            StructKey::Instance(family_name.to_string(), concrete_type.to_string()),
        );
        self.structs.insert(
            key,
            StructInfo {
                name: family_name.to_string(),
                fields: Vec::new(),
                alias_of: Some(concrete_type.to_string()),
                predicates: Some(predicates),
                is_instance_of: Some(family_name.to_string()),
                type_params: None,
            },
        );
        self.track_origin(family_name, origin);
    }

    /// Registra um struct paramétrico com type params.
    /// `data Complex (re::T im::T) where T implements SCALAR` registra
    /// `StructKey::Plain("Complex")` com `type_params: Some([...])` e
    /// fields contendo `Ty::Var("T")` nos tipos.
    pub fn register_generic(
        &mut self,
        origin: &str,
        name: &str,
        fields: Vec<FieldInfo>,
        type_params: Vec<TypeParamDecl>,
    ) {
        let key = (origin.to_string(), StructKey::Plain(name.to_string()));
        self.structs.insert(
            key,
            StructInfo {
                name: name.to_string(),
                fields,
                alias_of: None,
                predicates: None,
                is_instance_of: None,
                type_params: Some(type_params),
            },
        );
        self.track_origin(name, origin);
    }

    /// Instancia um struct paramétrico com type args concretos, substituindo
    /// `Ty::Var("T")` pelos types args nos tipos dos fields.
    /// Retorna `None` se o struct não existe ou não é paramétrico.
    ///
    /// O struct físico (layout, offsets) é idêntico — só os tipos anotados
    /// mudam para o type checker. O invariante `offset = i * 8` preserva-se.
    pub fn lookup_instantiated(
        &self,
        name: &str,
        type_args: &[Ty],
    ) -> Option<InstantiatedStructInfo> {
        let info = self.get(name)?;
        let type_params = info.type_params.as_ref()?;

        // Construir mapa de substituição: Var("T") → type_arg
        let mut subs: std::collections::HashMap<String, Ty> = std::collections::HashMap::new();
        for (param, arg) in type_params.iter().zip(type_args.iter()) {
            subs.insert(param.name.clone(), arg.clone());
        }

        // Substituir vars nos tipos dos fields
        let instantiated_fields: Vec<FieldInfo> = info
            .fields
            .iter()
            .map(|f| FieldInfo {
                name: f.name.clone(),
                ty: substitute_vars(&f.ty, &subs),
                offset: f.offset,
            })
            .collect();

        Some(InstantiatedStructInfo {
            name: name.to_string(),
            fields: instantiated_fields,
            type_args: type_args.to_vec(),
        })
    }

    /// Registra o mapeamento family_name → interface_name.
    /// Chamado em pass0 quando `data (IFACE, preds) as Fam` é processado,
    /// para permitir que `extend_families_for_implementors` encontre famílias
    /// sobre uma interface quando um novo implementor aparece.
    pub fn register_family_iface(&mut self, family_name: &str, iface_name: &str) {
        self.family_iface
            .insert(family_name.to_string(), iface_name.to_string());
    }

    /// Lista famílias polimórficas sobre uma interface.
    /// Retorna nomes de famílias cujo base (via `family_iface`) é a interface.
    pub fn families_over_iface(&self, iface: &str) -> Vec<String> {
        self.family_iface
            .iter()
            .filter(|(_, i)| *i == iface)
            .map(|(f, _)| f.clone())
            .collect()
    }

    /// true se `family::concrete` já foi registrada como instância.
    pub fn has_instance(&self, family: &str, concrete: &str) -> bool {
        self.get_instance(family, concrete).is_some()
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

    /// Verifica se um nome é uma família polimórfica (tem instâncias
    /// registradas com `is_instance_of: Some(name)`).
    pub fn is_family(&self, name: &str) -> bool {
        !self.all_instances(name).is_empty()
    }

    /// Busca informações de um struct pelo nome (não-qualificado).
    /// Retorna `None` se o nome é ambíguo ou não existe.
    /// Para famílias polimórficas, retorna a primeira instância encontrada
    /// (para obter instância específica, use `lookup` com type hint ou `get_instance`).
    pub fn get(&self, name: &str) -> Option<&StructInfo> {
        let origin = self.resolve_origin(name)?;
        let key = (origin.to_string(), StructKey::Plain(name.to_string()));
        // Tentar Plain primeiro (struct comum ou refined concreto).
        if let Some(info) = self.structs.get(&key) {
            return Some(info);
        }
        // Fallback: buscar qualquer Instance com este nome de família.
        self.structs
            .iter()
            .find(|((_, k), _)| k.name() == name && matches!(k, StructKey::Instance(..)))
            .map(|(_, info)| info)
    }

    /// `get` com origin explícita.
    pub fn get_with_origin(&self, origin: &str, name: &str) -> Option<&StructInfo> {
        let key = (origin.to_string(), StructKey::Plain(name.to_string()));
        self.structs.get(&key)
    }

    /// Lista todas as instâncias de uma família polimórfica.
    /// Retorna `(concrete_type, StructInfo)` para cada instância.
    /// `all_instances("NonZeroPoly")` → [("Int", info), ("Float", info), ("Rational", info)]
    ///
    /// Só retorna instâncias com `is_instance_of: Some` — refineds concretos
    /// (que têm `is_instance_of: None`) não são famílias polimórficas.
    pub fn all_instances(&self, family_name: &str) -> Vec<(&str, &StructInfo)> {
        self.structs
            .iter()
            .filter(|((_, k), info)| k.name() == family_name && info.is_instance_of.is_some())
            .filter_map(|(_, info)| info.alias_of.as_ref().map(|alias| (alias.as_str(), info)))
            .collect()
    }

    /// Busca uma instância específica de família pelo nome e tipo concreto.
    /// `get_instance("NonZero", "Int")` → StructInfo da instância NonZero/Int.
    pub fn get_instance(&self, family_name: &str, concrete_type: &str) -> Option<&StructInfo> {
        let origin = self.resolve_origin(family_name)?;
        let key = (
            origin.to_string(),
            StructKey::Instance(family_name.to_string(), concrete_type.to_string()),
        );
        self.structs.get(&key)
    }

    /// Lookup com type hint: resolve família → instância concreta.
    /// Se `name` é uma família e `type_hint` é `Some(Ty::Prim(PrimTy::Int))`,
    /// retorna a instância de NonZero para Int.
    /// Se `name` é um struct/refined comum, retorna como `get`.
    pub fn lookup(&self, name: &str, type_hint: Option<&Ty>) -> Option<&StructInfo> {
        // Se há type hint e o nome tem instâncias, tentar resolver instância.
        if let Some(hint) = type_hint {
            let concrete = match hint {
                Ty::Prim(crate::ty::PrimTy::Int) => Some("Int"),
                Ty::Prim(crate::ty::PrimTy::Float) => Some("Float"),
                Ty::Prim(crate::ty::PrimTy::Rational) => Some("Rational"),
                Ty::Prim(crate::ty::PrimTy::Text) => Some("Text"),
                Ty::Struct(crate::StructKey::Plain(n)) => Some(n.as_str()),
                _ => None,
            };
            #[allow(clippy::collapsible_if)]
            if let Some(concrete) = concrete {
                if let Some(instance) = self.get_instance(name, concrete) {
                    return Some(instance);
                }
            }
        }
        // Fallback: lookup como struct comum.
        self.get(name)
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

    /// Itera sobre todas as entradas (origin, StructKey, StructInfo).
    ///
    /// Usado pelo `TypeGraphBuilder` para classificar cada tipo registrado
    /// sem depender de nomes individuais.
    pub fn iter_all(&self) -> impl Iterator<Item = (&str, &StructKey, &StructInfo)> {
        self.structs
            .iter()
            .map(|((origin, key), info)| (origin.as_str(), key, info))
    }

    /// Lista os nomes de todas as famílias polimórficas registradas.
    ///
    /// Uma família é um nome que tem pelo menos uma instância
    /// (`is_instance_of: Some`). Derivado dos `origins` — para cada nome,
    /// verifica se existe pelo menos uma entrada com `is_instance_of: Some`.
    pub fn all_family_names(&self) -> Vec<String> {
        let mut families: BTreeSet<String> = BTreeSet::new();
        for ((_, key), info) in &self.structs {
            if info.is_instance_of.is_some() {
                families.insert(key.name().to_string());
            }
        }
        families.into_iter().collect()
    }

    // ── Merge ─────────────────────────────────────────────

    /// Mescla outro StructRegistry neste.
    /// Structs de origins diferentes coexistem; nomes com múltiplas origins
    /// são marcados como ambíguos. Structs da mesma origin são sobrescritos
    /// (re-registro no mesmo módulo).
    pub fn merge(&mut self, other: StructRegistry) {
        for ((origin, key), info) in other.structs {
            let k = (origin.clone(), key.clone());
            self.structs.insert(k, info);
            self.track_origin(key.name(), &origin);
        }
        // Mesclar family_iface — ambas as metades podem ter famílias.
        self.family_iface.extend(other.family_iface);
    }

    /// Filtra structs mantendo apenas aqueles cujo nome está no `closure`
    /// ou cuja origin é `core` (prelude). Usado por `filter_exports`.
    pub fn retain_by_closure(&mut self, closure: &std::collections::HashSet<String>) {
        self.structs.retain(|(_, key), _| {
            let name = key.name();
            closure.contains(name) || {
                self.origins
                    .get(name)
                    .is_some_and(|origins| origins.contains("core"))
            }
        });
        // Reconstruir origins e ambiguous
        self.origins.clear();
        self.ambiguous.clear();
        for (origin, key) in self.structs.keys() {
            let name = key.name();
            let origins = self.origins.entry(name.to_string()).or_default();
            origins.insert(origin.clone());
            if origins.len() > 1 {
                self.ambiguous.insert(name.to_string());
            }
        }
    }
}

/// Resultado de `lookup_instantiated` — struct paramétrico com type args
/// concretos aplicados aos tipos dos fields. O layout (offsets) é idêntico
/// ao do struct genérico; apenas os tipos anotados variam.
#[derive(Debug, Clone, PartialEq)]
pub struct InstantiatedStructInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    /// Type args concretos aplicados (ex: `[Int, Int]` para `Complex::(Int, Int)`).
    pub type_args: Vec<Ty>,
}

impl InstantiatedStructInfo {
    pub fn find_field(&self, name: &str) -> Option<(u32, &FieldInfo)> {
        self.fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == name)
            .map(|(i, f)| (i as u32, f))
    }
}

/// Substitui `Ty::Var(name)` por tipos concretos de `subs`, recursivamente.
/// Helper local para `lookup_instantiated` — não exporta para não conflitar
/// com `apply_subs` do inference (que tem assinatura diferente).
fn substitute_vars(ty: &Ty, subs: &std::collections::HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::Var(name) => subs.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Generic(name, args) => Ty::Generic(
            name.clone(),
            args.iter().map(|a| substitute_vars(a, subs)).collect(),
        ),
        Ty::Struct(StructKey::Generic(name, args)) => Ty::Struct(StructKey::Generic(
            name.clone(),
            args.iter().map(|a| substitute_vars(a, subs)).collect(),
        )),
        Ty::Function(params, ret) => Ty::Function(
            params.iter().map(|p| substitute_vars(p, subs)).collect(),
            Box::new(substitute_vars(ret, subs)),
        ),
        Ty::Tuple(elems) => {
            Ty::Tuple(elems.iter().map(|e| substitute_vars(e, subs)).collect())
        }
        Ty::List(e) => Ty::List(Box::new(substitute_vars(e, subs))),
        Ty::Array(e) => Ty::Array(Box::new(substitute_vars(e, subs))),
        _ => ty.clone(),
    }
}

#[cfg(test)]
#[path = "struct_registry_tests.rs"]
mod tests;
