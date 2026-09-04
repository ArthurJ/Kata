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

mod builder;

pub use builder::TypeGraphBuilder;

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
            let concrete = builder::prim_name(&resolved_params[0])?;
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Testes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
#[path = "../type_graph_tests.rs"]
mod tests;
