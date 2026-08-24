//! TypeGraph — classificação de tipos desacoplada de layout.
//!
//! O `StructRegistry` mistura duas responsabilidades com lifecycles diferentes:
//! - **Classificação** ("NonZero é família?") — necessária antes da resolução
//!   de assinaturas (Pass 1), quando o struct_registry do prelude ainda
//!   não foi merged.
//! - **Layout** ("quais campos Pessoa tem?") — necessária depois, no
//!   inference e codegen.
//!
//! O `TypeGraph` separa classificação de layout. É construído em duas fases:
//! 1. Prelude (embedded, conhecido em compile time) — disponível antes de
//!    qualquer resolução de módulo do usuário.
//! 2. Módulo do usuário (Pass 0) — estendido com declarações locais.
//!
//! Consultas de classificação (`kind_of`, `is_family`, `instances_of`,
//! `alias_target`, `refines_interfaces`) consultam o grafo, não os
//! registries. Consultas de layout (campos, predicados, offsets) continuam
//! no `StructRegistry`.
//!
//! ## Relações capturadas
//!
//! ```text
//! NonZero ──instance──→ NonZero::Int ──alias──→ Int
//!         ──instance──→ NonZero::Float ──alias──→ Float
//!
//! PositiveInt ──alias──→ Int ──refines──→ NUM
//!
//! Result ──generic_param──→ T, E
//!        ──variant──→ Ok(T), Err(E)
//!
//! Int ──implements──→ NUM, EQ, ORD, SHOW
//! ```

use std::collections::{HashMap, HashSet};

use crate::ty::{PrimTy, Ty};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Node
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Identificador único de um nó do grafo.
///
/// Usa `String` em vez de `&str` para que o grafo seja self-contained
/// (não empresta lifetime dos registries). O custo de alocação é pago
/// uma vez na construção e compensado pelas consultas O(1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeId(pub String);

/// Classificação de um tipo conhecido pelo compilador.
///
/// Determina como `resolve_type_expr` traduz `Name::Args` para `Ty`:
/// - `Family` → `Instance` quando o arg resolve para uma instância concreta.
/// - `GenericEnum` → `Generic` (aplicação de construtor de tipos com variantes).
/// - Demais → `Plain` / `Prim` / `Sum` / etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeKind {
    /// Primitivo: Int, Float, Text, Rational.
    Primitive(PrimTy),

    /// Struct com campos: `data Pessoa (nome::Text idade::Int)`.
    ///
    /// Layout (campos) fica no `StructRegistry`.
    Struct,

    /// Família polimórfica: `data (NUM, ...) as NonZero`.
    /// Tem instâncias registradas para tipos concretos.
    /// `instances` lista os tipos concretos (ex: `["Int", "Float"]`).
    Family { instances: Vec<String> },

    /// Refined concreto: `data (Int, > _ 0) as PositiveInt`.
    /// Tem predicados e alias para um tipo base.
    /// `alias_target` é o tipo base (ex: `"Int"`).
    /// `predicates` são os nomes das funções predicado no DispatchTable.
    Refined {
        alias_target: String,
        predicates: Vec<String>,
    },

    /// Enum genérico: `enum Result { Ok(T), Err(E) }`.
    /// Tem type params e variantes que dependem deles.
    GenericEnum { type_params: Vec<String> },

    /// Enum monomórfico: `enum Boolean { True, False }`.
    Enum,

    /// Interface: `interface NUM implements EQ`.
    /// `supertraits` lista interfaces pai.
    Interface { supertraits: Vec<String> },

    /// Type param: `T`, `E`, `A` (UPPER_CASE, não-resolvido).
    TypeParam,

    /// Alias (newtype): `alias Float as Altura`.
    /// `target` é o tipo apontado.
    Alias { target: String },

    /// Unit, Byte, Bytes, File, Socket — tipos intrínsecos sem parametrização.
    Intrinsic,
}

/// Um nó do grafo de tipos.
#[derive(Debug, Clone)]
pub struct TypeNode {
    pub id: TypeId,
    pub kind: TypeKind,
    /// Origem (módulo): `"core"` para prelude, `"__local__"` para user, etc.
    pub origin: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Edge
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Relação entre dois tipos no grafo.
///
/// Arestas são direcionadas e carregam metadados mínimos. O grafo é
/// multicampo — dois nós podem ter múltiplas arestas de tipos diferentes
/// (ex: PositiveInt ──alias──→ Int E ──refines──→ NUM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeEdge {
    /// Família → instância concreta.
    /// `NonZero ──instance──→ Int` (a instância NonZero::Int existe).
    /// `metadata` = nome concreto (ex: `"Int"`).
    Instance { concrete: String },

    /// Alias → tipo apontado.
    /// `Altura ──alias──→ Float`.
    /// `PositiveInt ──alias──→ Int`.
    Alias,

    /// Refined → interface delegada.
    /// `PositiveInt ──refines──→ NUM`.
    /// `metadata` = nome da interface.
    Refines { interface: String },

    /// Tipo concreto → interface implementada.
    /// `Int ──implements──→ NUM`.
    /// `metadata` = nome da interface.
    Implements { interface: String },

    /// Interface → supertrait.
    /// `NUM ──supertrait──→ EQ`.
    /// `metadata` = nome da supertrait.
    Supertrait { name: String },

    /// Enum genérico → type param.
    /// `Result ──generic_param──→ T`.
    /// `metadata` = nome do param.
    GenericParam { name: String },
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TypeGraph
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Grafo de classificação de tipos.
///
/// Self-contained — não empresta referências dos registries. Construído
/// a partir dos registries no Pass 0, mas depois opera independentemente.
///
/// ## Lifecycle
///
/// 1. `TypeGraph::prelude()` — constrói a partir do `StructRegistry`,
///    `InterfaceRegistry`, e `RefinesRegistry` do prelude (embedded).
///    Disponível antes da resolução de qualquer módulo do usuário.
/// 2. `graph.extend_with(&struct_reg, &iface_reg, &refines_reg, "__local__")`
///    — chamado no Pass 0 do módulo do usuário para adicionar declarações locais.
/// 3. Consultado durante Pass 1 (resolução de assinaturas) e inference.
///
/// ## Consultas
///
/// - `kind_of("NonZero")` → `Family { instances: ["Int", "Float", "Rational"] }`
/// - `instances_of("NonZero")` → `["Int", "Float", "Rational"]`
/// - `alias_target("PositiveInt")` → `Some("Int")`
/// - `follow_alias("Peso")` → `"Int"` (percorre cadeia: Peso → PositiveFloat → Float)
/// - `refines_interfaces("PositiveInt")` → `["NUM"]`
/// - `implements("Int")` → `["NUM", "EQ", "ORD", "SHOW"]`
/// - `is_family("NonZero")` → `true`
/// - `is_generic_enum("Result")` → `true`
#[derive(Debug, Clone, Default)]
pub struct TypeGraph {
    /// Nós indexados por nome.
    nodes: HashMap<String, TypeNode>,

    /// Arestas de saída indexadas por nó de origem.
    /// `edges["NonZero"] = [(Instance{concrete:"Int"}, "Int"), ...]`
    edges: HashMap<String, Vec<(TypeEdge, String)>>,

    /// Nomes ambíguos (definidos em múltiplas origins).
    ambiguous: HashSet<String>,
}

impl TypeGraph {
    // ── Construção ──────────────────────────────────────

    /// Cria grafo vazio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adiciona um nó ao grafo.
    ///
    /// Se já existe um nó com o mesmo nome e mesma origin, é no-op
    /// (idempotente). Se existe com origin diferente, marca como ambíguo.
    pub fn insert(&mut self, name: &str, kind: TypeKind, origin: &str) {
        let existing = self.nodes.get(name);
        match existing {
            Some(node) if node.origin == origin => {
                // Mesma origin — já registrado. No-op.
            }
            Some(_) => {
                // Origin diferente — ambíguo.
                self.ambiguous.insert(name.to_string());
                // Sobrescreve com a nova origin (preferência pelo último).
                // Em prática, o prelude vem primeiro e o user sobrescreve.
                self.nodes.insert(
                    name.to_string(),
                    TypeNode {
                        id: TypeId(name.to_string()),
                        kind,
                        origin: origin.to_string(),
                    },
                );
            }
            None => {
                self.nodes.insert(
                    name.to_string(),
                    TypeNode {
                        id: TypeId(name.to_string()),
                        kind,
                        origin: origin.to_string(),
                    },
                );
            }
        }
    }

    /// Adiciona uma aresta direcionada.
    pub fn add_edge(&mut self, from: &str, edge: TypeEdge, to: &str) {
        self.edges
            .entry(from.to_string())
            .or_default()
            .push((edge, to.to_string()));
    }

    // ── Consultas de classificação ───────────────────────

    /// Classificação de um tipo pelo nome.
    pub fn kind_of(&self, name: &str) -> Option<&TypeKind> {
        self.nodes.get(name).map(|n| &n.kind)
    }

    /// `true` se `name` é uma família polimórfica (tem instâncias).
    pub fn is_family(&self, name: &str) -> bool {
        matches!(self.kind_of(name), Some(TypeKind::Family { .. }))
    }

    /// `true` se `name` é um enum genérico (tem type params).
    pub fn is_generic_enum(&self, name: &str) -> bool {
        matches!(self.kind_of(name), Some(TypeKind::GenericEnum { .. }))
    }

    /// `true` se `name` é uma interface registrada.
    pub fn is_interface(&self, name: &str) -> bool {
        matches!(self.kind_of(name), Some(TypeKind::Interface { .. }))
    }

    /// `true` se `name` é um refined concreto (tem predicados).
    pub fn is_refined(&self, name: &str) -> bool {
        matches!(self.kind_of(name), Some(TypeKind::Refined { .. }))
    }

    /// `true` se `name` é um alias (newtype).
    pub fn is_alias(&self, name: &str) -> bool {
        matches!(self.kind_of(name), Some(TypeKind::Alias { .. }))
    }

    /// Lista as instâncias de uma família (ex: `["Int", "Float", "Rational"]`).
    pub fn instances_of(&self, family: &str) -> Vec<&str> {
        match self.kind_of(family) {
            Some(TypeKind::Family { instances }) => instances.iter().map(|s| s.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    /// Verifica se uma instância específica existe.
    /// `has_instance("NonZero", "Int")` → `true`.
    pub fn has_instance(&self, family: &str, concrete: &str) -> bool {
        self.instances_of(family).contains(&concrete)
    }

    /// Tipo alvo de um alias, se existir.
    /// `alias_target("Altura")` → `Some("Float")`.
    /// `alias_target("PositiveInt")` → `Some("Int")`.
    pub fn alias_target(&self, name: &str) -> Option<&str> {
        match self.kind_of(name) {
            Some(TypeKind::Alias { target }) => Some(target.as_str()),
            Some(TypeKind::Refined { alias_target, .. }) => Some(alias_target.as_str()),
            _ => None,
        }
    }

    /// Percorre a cadeia de alias até o tipo base final.
    /// `follow_alias("Peso")` → `"Float"` (Peso → PositiveFloat → Float).
    /// Retorna o nome original se não é alias.
    pub fn follow_alias(&self, name: &str) -> String {
        let mut current = name.to_string();
        let mut visited = HashSet::new();
        while let Some(target) = self.alias_target(&current) {
            if visited.contains(&current) {
                // Ciclo — retorna onde estamos.
                break;
            }
            visited.insert(current.clone());
            current = target.to_string();
        }
        current
    }

    /// Interfaces que um tipo refined delega via `refines`.
    /// `refines_interfaces("PositiveInt")` → `["NUM"]`.
    pub fn refines_interfaces(&self, name: &str) -> Vec<String> {
        self.edges
            .get(name)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|(e, _)| match e {
                        TypeEdge::Refines { interface } => Some(interface.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Interfaces que um tipo implementa diretamente.
    /// `implements("Int")` → `["NUM", "EQ", "ORD", "SHOW"]`.
    pub fn implements(&self, name: &str) -> Vec<String> {
        self.edges
            .get(name)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|(e, _)| match e {
                        TypeEdge::Implements { interface } => Some(interface.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Verifica se `type_name` implementa `interface_name` (direto ou herdado).
    pub fn type_implements(&self, type_name: &str, interface_name: &str) -> bool {
        let direct = self.implements(type_name);
        if direct.iter().any(|i| i == interface_name) {
            return true;
        }
        // Verifica supertraits recursivamente.
        for iface in &direct {
            if self.interface_has_method(iface, interface_name) {
                return true;
            }
        }
        false
    }

    /// Verifica se uma interface tem um método (direto ou herdado via supertraits).
    /// Usado para verificar se interface `iface` herda de `super`.
    pub fn interface_has_method(&self, iface: &str, super_name: &str) -> bool {
        let mut visited = HashSet::new();
        let mut queue = vec![iface.to_string()];
        while let Some(current) = queue.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current.clone());
            // Verifica supertraits de `current`.
            if let Some(edges) = self.edges.get(&current) {
                for (edge, target) in edges {
                    if let TypeEdge::Supertrait { name } = edge {
                        if name == super_name {
                            return true;
                        }
                        queue.push(target.clone());
                    }
                }
            }
        }
        false
    }

    /// `true` se o nome é ambíguo (definido em múltiplas origins).
    pub fn is_ambiguous(&self, name: &str) -> bool {
        self.ambiguous.contains(name)
    }

    /// Type params de um enum genérico.
    pub fn type_params_of(&self, name: &str) -> Vec<String> {
        match self.kind_of(name) {
            Some(TypeKind::GenericEnum { type_params }) => type_params.clone(),
            _ => Vec::new(),
        }
    }

    // ── Merge ───────────────────────────────────────────

    /// Merge de outro grafo (do prelude) neste.
    ///
    /// Nós do `other` que não existem localmente são adicionados.
    /// Nós que já existem localmente prevalecem (shadow do user sobre prelude).
    pub fn merge(&mut self, other: &TypeGraph) {
        for (name, node) in &other.nodes {
            if !self.nodes.contains_key(name) {
                self.nodes.insert(name.clone(), node.clone());
            }
        }
        for (from, edges) in &other.edges {
            self.edges
                .entry(from.clone())
                .or_default()
                .extend(edges.iter().cloned());
        }
        for name in &other.ambiguous {
            self.ambiguous.insert(name.clone());
        }
    }

    // ── Debug ───────────────────────────────────────────

    /// Debug: lista todos os nós com suas arestas.
    #[allow(dead_code)]
    pub fn debug_dump(&self) -> String {
        let mut out = String::new();
        let mut names: Vec<_> = self.nodes.keys().collect();
        names.sort();
        for name in names {
            let node = &self.nodes[name];
            out.push_str(&format!(
                "  {} [{:?}] origin={}\n",
                name, node.kind, node.origin
            ));
            if let Some(edges) = self.edges.get(name) {
                for (edge, to) in edges {
                    out.push_str(&format!("    ──{:?}──→ {}\n", edge, to));
                }
            }
        }
        out
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Construção a partir dos registries
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Builder que converte `StructRegistry` + `EnumRegistry` + `InterfaceRegistry`
/// + `RefinesRegistry` em um `TypeGraph`.
///
/// Cada registry contribui com um aspecto da classificação:
/// - `StructRegistry` → Struct, Family, Refined, Alias
/// - `EnumRegistry` → GenericEnum, Enum
/// - `InterfaceRegistry` → Interface, Implements, Supertrait
/// - `RefinesRegistry` → Refines
pub struct TypeGraphBuilder<'a> {
    pub struct_reg: &'a crate::StructRegistry,
    pub enum_reg: &'a crate::EnumRegistry,
    pub iface_reg: &'a crate::InterfaceRegistry,
    pub refines_reg: &'a crate::RefinesRegistry,
}

impl<'a> TypeGraphBuilder<'a> {
    pub fn build(&self, origin: &str) -> TypeGraph {
        let mut graph = TypeGraph::new();

        // ── Primitivos do prelude ──
        // Sempre presentes, independentemente do origin.
        graph.insert("Int", TypeKind::Primitive(PrimTy::Int), "core");
        graph.insert("Float", TypeKind::Primitive(PrimTy::Float), "core");
        graph.insert("Text", TypeKind::Primitive(PrimTy::Text), "core");
        graph.insert("Rational", TypeKind::Primitive(PrimTy::Rational), "core");
        graph.insert("Unit", TypeKind::Intrinsic, "core");
        graph.insert("Byte", TypeKind::Intrinsic, "core");
        graph.insert("Bytes", TypeKind::Intrinsic, "core");
        graph.insert("Boolean", TypeKind::Enum, "core");

        // ── StructRegistry → Struct, Family, Refined, Alias ──
        self.populate_from_struct_registry(&mut graph, origin);

        // ── EnumRegistry → GenericEnum, Enum ──
        self.populate_from_enum_registry(&mut graph, origin);

        // ── InterfaceRegistry → Interface, Implements, Supertrait ──
        self.populate_from_interface_registry(&mut graph, origin);

        // ── RefinesRegistry → Refines ──
        self.populate_from_refines_registry(&mut graph, origin);

        graph
    }

    fn populate_from_struct_registry(&self, graph: &mut TypeGraph, origin: &str) {
        // Primeiro: registrar famílias (antes das instâncias, para que
        // as arestas Instance possam referenciar o nó Family).
        for family_name in self.struct_reg.all_family_names() {
            let instances: Vec<String> = self
                .struct_reg
                .all_instances(&family_name)
                .iter()
                .map(|(concrete, _)| concrete.to_string())
                .collect();
            graph.insert(
                &family_name,
                TypeKind::Family {
                    instances: instances.clone(),
                },
                origin,
            );
            for concrete in &instances {
                graph.add_edge(
                    &family_name,
                    TypeEdge::Instance {
                        concrete: concrete.clone(),
                    },
                    concrete,
                );
            }
        }

        // Segundo: iterar sobre todas as entradas para registrar
        // Structs, Refineds, Aliases. Pular instâncias de família
        // (is_instance_of.is_some()) — já cobertas acima.
        for (_entry_origin, key, info) in self.struct_reg.iter_all() {
            // Instâncias de família: já registradas como arestas.
            if info.is_instance_of.is_some() {
                continue;
            }

            let name = key.name();

            // Refined concreto: tem predicates e alias_of.
            if info.predicates.is_some() {
                let alias_target = info.alias_of.clone().unwrap_or_default();
                graph.insert(
                    name,
                    TypeKind::Refined {
                        alias_target: alias_target.clone(),
                        predicates: info.predicates.clone().unwrap_or_default(),
                    },
                    origin,
                );
                if !alias_target.is_empty() {
                    graph.add_edge(name, TypeEdge::Alias, &alias_target);
                }
                continue;
            }

            // Alias (newtype): tem alias_of sem predicates.
            if let Some(ref alias) = info.alias_of {
                graph.insert(
                    name,
                    TypeKind::Alias {
                        target: alias.clone(),
                    },
                    origin,
                );
                graph.add_edge(name, TypeEdge::Alias, alias);
                continue;
            }

            // Struct com campos.
            if !info.fields.is_empty() {
                graph.insert(name, TypeKind::Struct, origin);
                continue;
            }

            // Struct sem campos e sem alias — tipo opaco (ex: data Int () @ffi).
            // Não registra no grafo — primitivos já estão acima.
        }
    }

    fn populate_from_enum_registry(&self, graph: &mut TypeGraph, origin: &str) {
        for name in self.enum_reg.names() {
            if self.enum_reg.is_generic(name) {
                // Enum genérico: tem type params.
                let type_params: Vec<String> = self
                    .enum_reg
                    .type_params_of(name)
                    .map(|tps| tps.to_vec())
                    .unwrap_or_default();
                graph.insert(name, TypeKind::GenericEnum { type_params }, origin);
                // Adiciona arestas para cada type param.
                for tp in self.enum_reg.type_params_of(name).unwrap_or(&[]) {
                    graph.add_edge(
                        name,
                        TypeEdge::GenericParam {
                            name: tp.to_string(),
                        },
                        tp,
                    );
                }
            } else {
                // Enum monomórfico.
                graph.insert(name, TypeKind::Enum, origin);
            }
        }
    }

    fn populate_from_interface_registry(&self, graph: &mut TypeGraph, origin: &str) {
        // Interfaces.
        for iface in self.iface_reg.all_interfaces() {
            graph.insert(
                &iface.name,
                TypeKind::Interface {
                    supertraits: iface.supertraits.clone(),
                },
                origin,
            );
            for st in &iface.supertraits {
                graph.add_edge(&iface.name, TypeEdge::Supertrait { name: st.clone() }, st);
            }
        }

        // Implementations.
        for impl_entry in self.iface_reg.impls_view() {
            graph.add_edge(
                &impl_entry.type_name,
                TypeEdge::Implements {
                    interface: impl_entry.interface_name.clone(),
                },
                &impl_entry.interface_name,
            );
        }
    }

    fn populate_from_refines_registry(&self, graph: &mut TypeGraph, _origin: &str) {
        for entry in self.refines_reg.all_entries() {
            graph.add_edge(
                &entry.type_name,
                TypeEdge::Refines {
                    interface: entry.interface_name.clone(),
                },
                &entry.interface_name,
            );
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Integração com resolve_type_expr (exemplo de uso)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Resolve `ParamApp { name, params }` usando o grafo em vez do struct_registry.
///
/// Esta é a função que substitui o bloco em `type_resolve.rs:236-256`.
/// Com o grafo, a classificação está disponível antes do merge do prelude.
pub fn resolve_param_app(name: &str, resolved_params: &[Ty], graph: &TypeGraph) -> Option<Ty> {
    match graph.kind_of(name) {
        // Família polimórfica: NonZero::Int → Instance("NonZero", "Int").
        Some(TypeKind::Family { instances }) if resolved_params.len() == 1 => {
            let concrete = prim_name(&resolved_params[0])?;
            if instances.iter().any(|i| i == &concrete) {
                return Some(Ty::Struct(crate::StructKey::Instance(
                    name.to_string(),
                    concrete,
                )));
            }
            None
        }
        // Enum genérico: Result::(Int, Text) → Generic("Result", [Int, Text]).
        Some(TypeKind::GenericEnum { .. }) => {
            Some(Ty::Generic(name.to_string(), resolved_params.to_vec()))
        }
        // Coleções intrínsecas (List, Array, etc.) continuam com match
        // hardcoded no caller — não é responsabilidade do grafo.
        _ => None,
    }
}

/// Extrai o nome de primitivo de um `Ty` para comparação de instâncias.
fn prim_name(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Prim(PrimTy::Int) => Some("Int".to_string()),
        Ty::Prim(PrimTy::Float) => Some("Float".to_string()),
        Ty::Prim(PrimTy::Rational) => Some("Rational".to_string()),
        Ty::Prim(PrimTy::Text) => Some("Text".to_string()),
        Ty::Struct(key) => Some(key.name().to_string()),
        _ => None,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Testes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_graph() -> TypeGraph {
        let mut graph = TypeGraph::new();

        // Primitivos
        graph.insert("Int", TypeKind::Primitive(PrimTy::Int), "core");
        graph.insert("Float", TypeKind::Primitive(PrimTy::Float), "core");

        // NonZero — família polimórfica
        graph.insert(
            "NonZero",
            TypeKind::Family {
                instances: vec!["Int".into(), "Float".into(), "Rational".into()],
            },
            "core",
        );
        graph.add_edge(
            "NonZero",
            TypeEdge::Instance {
                concrete: "Int".into(),
            },
            "Int",
        );
        graph.add_edge(
            "NonZero",
            TypeEdge::Instance {
                concrete: "Float".into(),
            },
            "Float",
        );

        // PositiveInt — refined concreto
        graph.insert(
            "PositiveInt",
            TypeKind::Refined {
                alias_target: "Int".into(),
                predicates: vec!["__pred_PositiveInt_0".into()],
            },
            "core",
        );
        graph.add_edge("PositiveInt", TypeEdge::Alias, "Int");

        // Altura — alias
        graph.insert(
            "Altura",
            TypeKind::Alias {
                target: "Float".into(),
            },
            "core",
        );
        graph.add_edge("Altura", TypeEdge::Alias, "Float");

        // Peso — alias de PositiveFloat (cadeia)
        graph.insert(
            "Peso",
            TypeKind::Alias {
                target: "PositiveFloat".into(),
            },
            "user",
        );
        graph.add_edge("Peso", TypeEdge::Alias, "PositiveFloat");
        graph.insert(
            "PositiveFloat",
            TypeKind::Refined {
                alias_target: "Float".into(),
                predicates: vec!["__pred_PositiveFloat_0".into()],
            },
            "core",
        );
        graph.add_edge("PositiveFloat", TypeEdge::Alias, "Float");

        // Result — enum genérico
        graph.insert(
            "Result",
            TypeKind::GenericEnum {
                type_params: vec!["T".into(), "E".into()],
            },
            "core",
        );
        graph.add_edge("Result", TypeEdge::GenericParam { name: "T".into() }, "T");
        graph.add_edge("Result", TypeEdge::GenericParam { name: "E".into() }, "E");

        // NUM — interface
        graph.insert(
            "NUM",
            TypeKind::Interface {
                supertraits: vec!["ORD".into(), "EQ".into()],
            },
            "core",
        );
        graph.add_edge("NUM", TypeEdge::Supertrait { name: "ORD".into() }, "ORD");
        graph.add_edge("NUM", TypeEdge::Supertrait { name: "EQ".into() }, "EQ");

        // Int implements NUM
        graph.add_edge(
            "Int",
            TypeEdge::Implements {
                interface: "NUM".into(),
            },
            "NUM",
        );

        // PositiveInt refines NUM
        graph.add_edge(
            "PositiveInt",
            TypeEdge::Refines {
                interface: "NUM".into(),
            },
            "NUM",
        );

        graph
    }

    #[test]
    fn classify_family() {
        let graph = build_test_graph();
        assert!(graph.is_family("NonZero"));
        assert!(!graph.is_family("Int"));
        assert!(!graph.is_family("Result"));
    }

    #[test]
    fn classify_generic_enum() {
        let graph = build_test_graph();
        assert!(graph.is_generic_enum("Result"));
        assert!(!graph.is_generic_enum("NonZero"));
    }

    #[test]
    fn instances_of_family() {
        let graph = build_test_graph();
        let instances = graph.instances_of("NonZero");
        assert_eq!(instances, vec!["Int", "Float", "Rational"]);
        assert!(graph.has_instance("NonZero", "Int"));
        assert!(!graph.has_instance("NonZero", "Text"));
    }

    #[test]
    fn alias_target_single() {
        let graph = build_test_graph();
        assert_eq!(graph.alias_target("Altura"), Some("Float"));
        assert_eq!(graph.alias_target("PositiveInt"), Some("Int"));
        assert_eq!(graph.alias_target("Int"), None);
    }

    #[test]
    fn follow_alias_chain() {
        let graph = build_test_graph();
        // Peso → PositiveFloat → Float
        assert_eq!(graph.follow_alias("Peso"), "Float");
        // PositiveInt → Int (refined tem alias_target)
        assert_eq!(graph.follow_alias("PositiveInt"), "Int");
        // Int — não é alias, retorna ele mesmo
        assert_eq!(graph.follow_alias("Int"), "Int");
    }

    #[test]
    fn refines_interfaces() {
        let graph = build_test_graph();
        let ifaces = graph.refines_interfaces("PositiveInt");
        assert_eq!(ifaces, vec!["NUM"]);
    }

    #[test]
    fn implements_list() {
        let graph = build_test_graph();
        let ifaces = graph.implements("Int");
        assert_eq!(ifaces, vec!["NUM"]);
    }

    #[test]
    fn type_implements_direct() {
        let graph = build_test_graph();
        assert!(graph.type_implements("Int", "NUM"));
        assert!(!graph.type_implements("Int", "SHOW"));
    }

    #[test]
    fn resolve_param_app_family() {
        let graph = build_test_graph();
        let result = resolve_param_app("NonZero", &[Ty::Prim(PrimTy::Int)], &graph);
        assert_eq!(
            result,
            Some(Ty::Struct(crate::StructKey::Instance(
                "NonZero".into(),
                "Int".into()
            )))
        );
    }

    #[test]
    fn resolve_param_app_generic_enum() {
        let graph = build_test_graph();
        let result = resolve_param_app(
            "Result",
            &[Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Text)],
            &graph,
        );
        assert_eq!(
            result,
            Some(Ty::Generic(
                "Result".into(),
                vec![Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Text)]
            ))
        );
    }

    #[test]
    fn resolve_param_app_unknown() {
        let graph = build_test_graph();
        let result = resolve_param_app("Unknown", &[Ty::Prim(PrimTy::Int)], &graph);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_param_app_family_wrong_concrete() {
        let graph = build_test_graph();
        // Text não é instância de NonZero
        let result = resolve_param_app("NonZero", &[Ty::Prim(PrimTy::Text)], &graph);
        assert_eq!(result, None);
    }

    #[test]
    fn merge_graphs() {
        let mut prelude = build_test_graph();

        // User declara um novo refined
        let mut user = TypeGraph::new();
        user.insert(
            "MyRefined",
            TypeKind::Refined {
                alias_target: "Int".into(),
                predicates: vec!["__pred_MyRefined_0".into()],
            },
            "__local__",
        );

        prelude.merge(&user);
        assert!(prelude.is_refined("MyRefined"));
        assert_eq!(prelude.alias_target("MyRefined"), Some("Int"));
        // Original intacto
        assert!(prelude.is_family("NonZero"));
    }
}
