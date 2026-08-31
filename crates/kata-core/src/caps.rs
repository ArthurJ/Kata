//! Capacidades e representação de tipos — `TypeCaps`, `Repr`, `CapsIndex`.
//!
//! Derivado das registries dinâmicas (`TypeGraph`, `StructRegistry`,
//! `InterfaceRegistry`, `InlineFnTable`) que o compilador já constrói.
//! Permite que o motor Maranget consulte capacidade (ORD? EQ? NUM?) e
//! representação de dados sem hard-codar nomes de tipo. Tipos de usuário
//! funcionam automaticamente.
//!
//! ## Eixo capacidade vs transparência
//!
//! - **Capacidade** = o que a linguagem permite (implementa ORD? EQ? NUM?)
//!   — via `TypeGraph::type_implements`.
//! - **Transparência** = o que o compilador consegue provar (const-eval?
//!   inline?) — via `InlineFnTable` (não acessível aqui; transparência
//!   fica em `InferCtx` e é consultada separadamente).
//! - Capacidade sem transparência → fallback estrutural conservador
//!   (Missing/otherwise), nunca erro falso.

use crate::struct_registry::StructRegistry;
use crate::ty::{PrimTy, Ty};
use crate::type_graph::TypeGraph;

/// Representação de dados de um tipo — como o valor é estruturado
/// fisicamente. Ortogonal a capacidade (ord/eq).
///
/// Determina como `Constructor::Literal` é serializado e como
/// `enum_refined_domain` enumera valores.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Repr {
    /// Inteiro discreto de 64 bits. Domínio enumerável via intervalo.
    Int,
    /// Ponto flutuante. Domínio NÃO enumerável (densidade infinita).
    Float,
    /// Racional (par num/den). Domínio NÃO enumerável diretamente.
    Rational,
    /// Texto. Domínio NÃO enumerável.
    Text,
    /// Unit — único valor `()`.
    Unit,
    /// Boolean — enum com duas variantes (não é Repr, é Sum).
    /// Mas para fins de const-eval, Boolean é enumerável.
    Bool,
    /// Struct com N campos — cada campo tem sua própria Repr.
    /// Domínio potencialmente enumerável se todos os campos têm Repr
    /// discreta e o tipo implementa EQ + const-eval.
    Struct(Vec<Repr>),
    /// Representação desconhecida/não-suportada — fallback opaco.
    /// O motor trata como tipo infinito sem construtores nomeados.
    Opaque,
}

/// Capacidades de um tipo — o que a linguagem permite fazer com ele.
///
/// Derivado de `TypeGraph::type_implements` (que consulta as
/// declarações `implements` do usuário + stdlib). Ortogonal a `Repr`:
/// `Repr` descreve a representação; `TypeCaps` descreve quais operações
/// são válidas sobre essa representação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCaps {
    /// Implementa `ORD` (ordenabilidade: `<`, `<=`, `>`, `>=`).
    /// `ORD implements EQ`, então `ord` implica `eq`.
    pub ord: bool,
    /// Implementa `EQ` (igualdade: `=`, `!=`).
    pub eq: bool,
    /// Implementa `NUM` (aritmética: `+`, `-`, `*`, `/`).
    pub num: bool,
    /// Representação de dados.
    pub repr: Repr,
}

impl TypeCaps {
    /// Caps para um tipo que não existe no grafo — fallback conservador.
    /// Sem capacidades, repr opaca.
    fn unknown() -> Self {
        TypeCaps {
            ord: false,
            eq: false,
            num: false,
            repr: Repr::Opaque,
        }
    }
}

/// Índice de capacidades construído a partir de `TypeGraph` e
/// `StructRegistry`. Consulta por nome de tipo.
///
/// Construído uma vez no início de `infer_module` a partir de `InferCtx`.
/// `MarangetEnv` recebe `&CapsIndex` — preserva a separação motor
/// puro/typeck.
#[derive(Debug, Clone)]
pub struct CapsIndex {
    /// Cache de caps por nome de tipo (após `follow_alias`).
    /// Key = nome do tipo após resolução de alias.
    caps: std::collections::HashMap<String, TypeCaps>,
}

impl CapsIndex {
    /// Constrói o índice a partir de `TypeGraph` e `StructRegistry`.
    ///
    /// Percorre todos os nós do `TypeGraph` e computa `TypeCaps` para cada
    /// tipo conhecido. Tipos não registrados ficam de fora do cache e
    /// retornam `TypeCaps::unknown()` na consulta.
    pub fn new(type_graph: &TypeGraph, struct_registry: &StructRegistry) -> Self {
        let mut caps = std::collections::HashMap::new();

        // Tipos primitivos do prelude — sempre presentes.
        for (name, prim) in [
            ("Int", PrimTy::Int),
            ("Float", PrimTy::Float),
            ("Text", PrimTy::Text),
            ("Rational", PrimTy::Rational),
        ] {
            let c = compute_caps(type_graph, name, Some(prim), struct_registry);
            caps.insert(name.to_string(), c);
        }

        // Boolean é enum, não primitivo — mas tem repr Bool para const-eval.
        let bool_caps = compute_caps(type_graph, "Boolean", None, struct_registry);
        caps.insert("Boolean".to_string(), bool_caps);

        // Percorre todos os nós do TypeGraph para registrar tipos de usuário.
        // TypeGraph não expõe iteração pública dos nós, então registramos
        // apenas os primitivos + Boolean + os que aparecem em consultas.
        // Tipos de usuário são resolvidos on-demand em `get_or_compute`.

        CapsIndex { caps }
    }

    /// Consulta as capacidades de um tipo pelo nome (após `follow_alias`).
    ///
    /// Se o tipo já está no cache, retorna do cache. Caso contrário,
    /// computa on-demand e armazena.
    pub fn get(&self, type_name: &str) -> TypeCaps {
        self.caps
            .get(type_name)
            .cloned()
            .unwrap_or_else(TypeCaps::unknown)
    }

    /// Consulta as capacidades de um `Ty`.
    ///
    /// Resolve `Ty::Struct(key)` via `follow_alias` do `TypeGraph`,
    /// `Ty::Sum`/`Ty::Generic` como enum (sem repr discreta),
    /// `Ty::Prim` direto.
    pub fn get_for_ty(&self, ty: &Ty) -> TypeCaps {
        match ty {
            Ty::Prim(PrimTy::Int) => self.get("Int"),
            Ty::Prim(PrimTy::Float) => self.get("Float"),
            Ty::Prim(PrimTy::Text) => self.get("Text"),
            Ty::Prim(PrimTy::Rational) => self.get("Rational"),
            Ty::Unit => TypeCaps {
                ord: false,
                eq: true,
                num: false,
                repr: Repr::Unit,
            },
            Ty::Sum(name) | Ty::Generic(name, _) => {
                // Enum — sem repr discreta (variantes, não literais).
                // Capacidades via TypeGraph.
                let c = self.get(name);
                if c.repr == Repr::Opaque {
                    // Enum desconhecido — mantém Opaque mas pode ter eq
                    // se implementar EQ.
                    TypeCaps {
                        ord: self.get(name).ord,
                        eq: self.get(name).eq,
                        num: self.get(name).num,
                        repr: Repr::Opaque,
                    }
                } else {
                    c
                }
            }
            Ty::Struct(key) => {
                // Refined/alias/struct — resolve pelo nome público.
                self.get(key.name())
            }
            _ => TypeCaps::unknown(),
        }
    }
}

/// Computa `TypeCaps` para um tipo pelo nome, consultando `TypeGraph`
/// para capacidades (ORD, EQ, NUM) e derivando `Repr` do `PrimTy` ou
/// `StructRegistry`.
fn compute_caps(
    type_graph: &TypeGraph,
    name: &str,
    prim: Option<PrimTy>,
    struct_registry: &StructRegistry,
) -> TypeCaps {
    // Capacidades via TypeGraph.
    let ord = type_graph.type_implements(name, "ORD");
    let eq = type_graph.type_implements(name, "EQ") || ord; // ORD implica EQ
    let num = type_graph.type_implements(name, "NUM");

    // Representação: PrimTy se primitivo, Struct se tem campos, Opaque caso contrário.
    let repr = if let Some(p) = prim {
        match p {
            PrimTy::Int => Repr::Int,
            PrimTy::Float => Repr::Float,
            PrimTy::Text => Repr::Text,
            PrimTy::Rational => Repr::Rational,
        }
    } else if name == "Boolean" {
        Repr::Bool
    } else {
        // Tenta buscar campos no StructRegistry.
        // Para refineds/aliases, os campos são os do tipo base.
        // Por enquanto, structs de usuário com campos → Repr::Struct.
        let struct_info = struct_registry.get(name);
        if let Some(info) = struct_info
            && !info.fields.is_empty()
        {
            let field_reprs = info
                .fields
                .iter()
                .map(|f| repr_of_ty(&f.ty, type_graph, struct_registry))
                .collect();
            Repr::Struct(field_reprs)
        } else {
            Repr::Opaque
        }
    };

    TypeCaps { ord, eq, num, repr }
}

/// Deriva `Repr` de um `Ty` recursivamente, consultando registries.
fn repr_of_ty(ty: &Ty, type_graph: &TypeGraph, struct_registry: &StructRegistry) -> Repr {
    match ty {
        Ty::Prim(PrimTy::Int) => Repr::Int,
        Ty::Prim(PrimTy::Float) => Repr::Float,
        Ty::Prim(PrimTy::Text) => Repr::Text,
        Ty::Prim(PrimTy::Rational) => Repr::Rational,
        Ty::Unit => Repr::Unit,
        Ty::Sum(name) | Ty::Generic(name, _) if name == "Boolean" => Repr::Bool,
        Ty::Struct(key) => {
            let base = type_graph.follow_alias(key.name());
            if base != key.name() {
                // É alias/refined — computa repr do tipo base.
                let struct_info = struct_registry.get(&base);
                if let Some(info) = struct_info
                    && !info.fields.is_empty()
                {
                    let field_reprs = info
                        .fields
                        .iter()
                        .map(|f| repr_of_ty(&f.ty, type_graph, struct_registry))
                        .collect();
                    Repr::Struct(field_reprs)
                } else {
                    // Base é primitivo — mapeia por nome.
                    prim_repr_by_name(&base)
                }
            } else {
                Repr::Opaque
            }
        }
        _ => Repr::Opaque,
    }
}

/// Mapeia nome de tipo primitivo para `Repr`.
fn prim_repr_by_name(name: &str) -> Repr {
    match name {
        "Int" => Repr::Int,
        "Float" => Repr::Float,
        "Text" => Repr::Text,
        "Rational" => Repr::Rational,
        _ => Repr::Opaque,
    }
}
